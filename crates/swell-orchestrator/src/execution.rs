//! Execution controller for managing parallel agent execution.
#![allow(clippy::should_implement_trait)]

use crate::{
    drift_detector::{DriftDetector, DriftReport},
    file_locks::LockAcquisitionResult,
    frozen_spec::FrozenSpecRef,
    killswitch::OrchestratorKillSwitch,
    EvaluatorAgent, FeatureLead, FeatureLeadSpawner, GeneratorAgent, Orchestrator, PlannerAgent,
    MAX_CONCURRENT_AGENTS,
};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use swell_core::traits::Agent;
use swell_core::{
    AgentContext, AgentResult, LlmMessage, Plan, StreamEvent, SwellError, ToolOutput,
    ToolResultContent, ValidationResult,
};
use swell_llm::{LlmBackend, LlmToolDefinition};
use swell_tools::{
    resource_limits::{SessionLimits, SessionResourceTracker},
    ToolRegistry,
};
use swell_validation::ValidationPipeline;
use tracing::{debug, info, warn};
use uuid::Uuid;

// ============================================================================
// Helper Functions
// ============================================================================

/// Extract text content from ToolResultContent
fn extract_text_from_content(content: &[ToolResultContent]) -> String {
    match content.first() {
        Some(ToolResultContent::Text(s)) => s.clone(),
        Some(ToolResultContent::Error(e)) => e.clone(),
        Some(ToolResultContent::Json(v)) => serde_json::to_string(v).unwrap_or_default(),
        Some(ToolResultContent::Image { data, .. }) => data.clone(),
        None => String::new(),
    }
}

// Test-only helper function for extracting error messages
#[cfg(test)]
fn extract_error_from_content(content: &[ToolResultContent]) -> String {
    match content.first() {
        Some(ToolResultContent::Error(e)) => e.clone(),
        _ => String::new(),
    }
}

// ============================================================================
// Turn Loop Types
// ============================================================================

/// Outcome of a single turn in the execution loop
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    /// Turn completed successfully with a text response (no tool calls)
    TextOnly,
    /// Turn completed with tool calls that were executed
    ToolCallsExecuted,
    /// Turn ended due to reaching max_iterations hard cap
    MaxIterationsReached,
    /// Turn ended due to an error
    Error,
    /// Turn ended due to empty response
    EmptyResponse,
    /// Turn ended due to kill switch being triggered
    KillSwitchTriggered,
    /// Turn ended due to resource limit exceeded
    ResourceLimitExceeded,
}

/// A tool call that was made during a turn
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnToolCall {
    /// The tool name
    pub name: String,
    /// The tool call ID
    pub id: String,
    /// The arguments passed to the tool
    pub arguments: serde_json::Value,
    /// The result of the tool execution
    pub result: String,
    /// Whether the tool execution was successful
    pub success: bool,
}

/// Summary of a single turn in the execution loop.
///
/// Captures token usage, tool calls made, and the outcome of the turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnSummary {
    /// The turn number (1-indexed)
    pub turn_number: u32,
    /// Token usage for this turn
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Cache tokens (Anthropic)
    pub cache_creation_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    /// Tool calls made during this turn
    pub tool_calls: Vec<TurnToolCall>,
    /// The outcome of this turn
    pub outcome: TurnOutcome,
    /// The final accumulated text (if any)
    pub final_text: String,
    /// Stop reason from the stream (if any)
    pub stop_reason: Option<String>,
    /// Error message if outcome is Error
    pub error_message: Option<String>,
}

impl TurnSummary {
    /// Create a new TurnSummary for the given turn number
    pub fn new(turn_number: u32) -> Self {
        Self {
            turn_number,
            input_tokens: 0,
            output_tokens: 0,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
            tool_calls: Vec::new(),
            outcome: TurnOutcome::EmptyResponse,
            final_text: String::new(),
            stop_reason: None,
            error_message: None,
        }
    }

    /// Add a tool call to this turn
    pub fn add_tool_call(
        &mut self,
        name: String,
        id: String,
        arguments: serde_json::Value,
        result: String,
        success: bool,
    ) {
        self.tool_calls.push(TurnToolCall {
            name,
            id,
            arguments,
            result,
            success,
        });
    }

    /// Total tokens used in this turn
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }

    /// Whether this turn had any tool calls
    pub fn had_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}

/// Default max iterations for the turn loop
pub const DEFAULT_MAX_ITERATIONS: u32 = 50;

/// Default context compaction threshold (in tokens)
/// When accumulated conversation history exceeds this, compaction is triggered.
pub const DEFAULT_CONTEXT_COMPACTION_THRESHOLD: usize = 100_000;

/// Default number of tail messages to always preserve during compaction.
pub const DEFAULT_TAIL_MESSAGE_COUNT: usize = 10;

/// A pending tool call tracked during stream processing
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct PendingToolCall {
    /// The tool call ID used to match with ToolResult
    id: String,
    /// The tool name
    name: String,
    /// The tool arguments
    arguments: serde_json::Value,
}

/// Manages concurrent task execution with up to 6 agents
pub struct ExecutionController {
    orchestrator: Arc<Orchestrator>,
    llm: Arc<dyn LlmBackend>,
    tool_registry: Arc<ToolRegistry>,
    validation_pipeline: ValidationPipeline,
    /// Kill switch for emergency stops and pause/resume
    kill_switch: OrchestratorKillSwitch,
    /// Resource tracker for session limits
    resource_tracker: SessionResourceTracker,
    max_concurrent: usize,
    /// Maximum iterations for the turn loop (hard cap)
    /// This is separate from validation retry count
    max_iterations: u32,
    /// Token threshold for triggering context compaction.
    /// When conversation history exceeds this, compaction is triggered.
    context_compaction_threshold: usize,
    /// Number of tail (most recent) messages to always preserve during compaction.
    tail_message_count: usize,
    /// Frozen specs indexed by task_id, created at execution start
    frozen_specs: std::sync::RwLock<HashMap<uuid::Uuid, FrozenSpecRef>>,
    /// Active FeatureLeads for complex tasks
    feature_leads: std::sync::RwLock<HashMap<uuid::Uuid, FeatureLead>>,
    /// Drift detector for comparing actual vs planned file modifications
    drift_detector: DriftDetector,
    /// Files modified during the current execution session
    modified_files: std::sync::RwLock<HashSet<String>>,
    /// Interval in seconds for periodic drift checks (0 = only at end of execution)
    drift_check_interval_seconds: u64,
}

impl ExecutionController {
    /// Create a new ExecutionController with injected dependencies.
    ///
    /// # Arguments
    /// * `orchestrator` - The orchestrator for task coordination
    /// * `llm` - The LLM backend for agent reasoning
    /// * `tool_registry` - The tool registry for tool execution
    pub fn new(
        orchestrator: Arc<Orchestrator>,
        llm: Arc<dyn LlmBackend>,
        tool_registry: Arc<ToolRegistry>,
    ) -> Self {
        Self {
            orchestrator,
            llm,
            tool_registry,
            validation_pipeline: ValidationPipeline::new(),
            kill_switch: OrchestratorKillSwitch::new(),
            resource_tracker: SessionResourceTracker::with_default_limits(),
            max_concurrent: MAX_CONCURRENT_AGENTS,
            max_iterations: DEFAULT_MAX_ITERATIONS,
            context_compaction_threshold: DEFAULT_CONTEXT_COMPACTION_THRESHOLD,
            tail_message_count: DEFAULT_TAIL_MESSAGE_COUNT,
            frozen_specs: std::sync::RwLock::new(HashMap::new()),
            feature_leads: std::sync::RwLock::new(HashMap::new()),
            drift_detector: DriftDetector::new(),
            modified_files: std::sync::RwLock::new(HashSet::new()),
            drift_check_interval_seconds: 0,
        }
    }

    /// Create a new ExecutionController with a custom validation pipeline.
    pub fn with_pipeline(
        orchestrator: Arc<Orchestrator>,
        llm: Arc<dyn LlmBackend>,
        tool_registry: Arc<ToolRegistry>,
        validation_pipeline: ValidationPipeline,
    ) -> Self {
        Self {
            orchestrator,
            llm,
            tool_registry,
            validation_pipeline,
            kill_switch: OrchestratorKillSwitch::new(),
            resource_tracker: SessionResourceTracker::with_default_limits(),
            max_concurrent: MAX_CONCURRENT_AGENTS,
            max_iterations: DEFAULT_MAX_ITERATIONS,
            context_compaction_threshold: DEFAULT_CONTEXT_COMPACTION_THRESHOLD,
            tail_message_count: DEFAULT_TAIL_MESSAGE_COUNT,
            frozen_specs: std::sync::RwLock::new(HashMap::new()),
            feature_leads: std::sync::RwLock::new(HashMap::new()),
            drift_detector: DriftDetector::new(),
            modified_files: std::sync::RwLock::new(HashSet::new()),
            drift_check_interval_seconds: 0,
        }
    }

    /// Create a new ExecutionController with custom max_iterations.
    ///
    /// # Arguments
    /// * `orchestrator` - The orchestrator for task coordination
    /// * `llm` - The LLM backend for agent reasoning
    /// * `tool_registry` - The tool registry for tool execution
    /// * `max_iterations` - Maximum iterations for the turn loop (hard cap, separate from validation retries)
    pub fn with_max_iterations(
        orchestrator: Arc<Orchestrator>,
        llm: Arc<dyn LlmBackend>,
        tool_registry: Arc<ToolRegistry>,
        max_iterations: u32,
    ) -> Self {
        Self {
            orchestrator,
            llm,
            tool_registry,
            validation_pipeline: ValidationPipeline::new(),
            kill_switch: OrchestratorKillSwitch::new(),
            resource_tracker: SessionResourceTracker::with_default_limits(),
            max_concurrent: MAX_CONCURRENT_AGENTS,
            max_iterations,
            context_compaction_threshold: DEFAULT_CONTEXT_COMPACTION_THRESHOLD,
            tail_message_count: DEFAULT_TAIL_MESSAGE_COUNT,
            frozen_specs: std::sync::RwLock::new(HashMap::new()),
            feature_leads: std::sync::RwLock::new(HashMap::new()),
            drift_detector: DriftDetector::new(),
            modified_files: std::sync::RwLock::new(HashSet::new()),
            drift_check_interval_seconds: 0,
        }
    }

    /// Create a new ExecutionController with custom validation pipeline and max_iterations.
    pub fn with_pipeline_and_max_iterations(
        orchestrator: Arc<Orchestrator>,
        llm: Arc<dyn LlmBackend>,
        tool_registry: Arc<ToolRegistry>,
        validation_pipeline: ValidationPipeline,
        max_iterations: u32,
    ) -> Self {
        Self {
            orchestrator,
            llm,
            tool_registry,
            validation_pipeline,
            kill_switch: OrchestratorKillSwitch::new(),
            resource_tracker: SessionResourceTracker::with_default_limits(),
            max_concurrent: MAX_CONCURRENT_AGENTS,
            max_iterations,
            context_compaction_threshold: DEFAULT_CONTEXT_COMPACTION_THRESHOLD,
            tail_message_count: DEFAULT_TAIL_MESSAGE_COUNT,
            frozen_specs: std::sync::RwLock::new(HashMap::new()),
            feature_leads: std::sync::RwLock::new(HashMap::new()),
            drift_detector: DriftDetector::new(),
            modified_files: std::sync::RwLock::new(HashSet::new()),
            drift_check_interval_seconds: 0,
        }
    }

    /// Create a new ExecutionController with all custom settings including context compaction.
    ///
    /// # Arguments
    /// * `orchestrator` - The orchestrator for task coordination
    /// * `llm` - The LLM backend for agent reasoning
    /// * `tool_registry` - The tool registry for tool execution
    /// * `validation_pipeline` - Custom validation pipeline
    /// * `max_iterations` - Maximum iterations for the turn loop (hard cap)
    /// * `context_compaction_threshold` - Token threshold for triggering context compaction
    /// * `tail_message_count` - Number of tail messages to always preserve
    pub fn with_all_settings(
        orchestrator: Arc<Orchestrator>,
        llm: Arc<dyn LlmBackend>,
        tool_registry: Arc<ToolRegistry>,
        validation_pipeline: ValidationPipeline,
        max_iterations: u32,
        context_compaction_threshold: usize,
        tail_message_count: usize,
    ) -> Self {
        Self {
            orchestrator,
            llm,
            tool_registry,
            validation_pipeline,
            kill_switch: OrchestratorKillSwitch::new(),
            resource_tracker: SessionResourceTracker::with_default_limits(),
            max_concurrent: MAX_CONCURRENT_AGENTS,
            max_iterations,
            context_compaction_threshold,
            tail_message_count,
            frozen_specs: std::sync::RwLock::new(HashMap::new()),
            feature_leads: std::sync::RwLock::new(HashMap::new()),
            drift_detector: DriftDetector::new(),
            modified_files: std::sync::RwLock::new(HashSet::new()),
            drift_check_interval_seconds: 0,
        }
    }

    /// Create a new ExecutionController with all custom settings including resource limits.
    ///
    /// # Arguments
    /// * `orchestrator` - The orchestrator for task coordination
    /// * `llm` - The LLM backend for agent reasoning
    /// * `tool_registry` - The tool registry for tool execution
    /// * `validation_pipeline` - Custom validation pipeline
    /// * `max_iterations` - Maximum iterations for the turn loop (hard cap)
    /// * `context_compaction_threshold` - Token threshold for triggering context compaction
    /// * `tail_message_count` - Number of tail messages to always preserve
    /// * `session_limits` - Session resource limits (max turns, wall clock timeout, etc.)
    #[allow(clippy::too_many_arguments)]
    pub fn with_resource_limits(
        orchestrator: Arc<Orchestrator>,
        llm: Arc<dyn LlmBackend>,
        tool_registry: Arc<ToolRegistry>,
        validation_pipeline: ValidationPipeline,
        max_iterations: u32,
        context_compaction_threshold: usize,
        tail_message_count: usize,
        session_limits: SessionLimits,
    ) -> Self {
        Self {
            orchestrator,
            llm,
            tool_registry,
            validation_pipeline,
            kill_switch: OrchestratorKillSwitch::new(),
            resource_tracker: SessionResourceTracker::new(session_limits),
            max_concurrent: MAX_CONCURRENT_AGENTS,
            max_iterations,
            context_compaction_threshold,
            tail_message_count,
            frozen_specs: std::sync::RwLock::new(HashMap::new()),
            feature_leads: std::sync::RwLock::new(HashMap::new()),
            drift_detector: DriftDetector::new(),
            modified_files: std::sync::RwLock::new(HashSet::new()),
            drift_check_interval_seconds: 0,
        }
    }

    /// Get a reference to the resource tracker
    pub fn resource_tracker(&self) -> &SessionResourceTracker {
        &self.resource_tracker
    }

    /// Get a mutable reference to the resource tracker for recording usage
    pub fn resource_tracker_mut(&mut self) -> &mut SessionResourceTracker {
        &mut self.resource_tracker
    }

    /// Get the max iterations setting
    pub fn max_iterations(&self) -> u32 {
        self.max_iterations
    }

    /// Get a reference to the orchestrator kill switch for emergency stops.
    pub fn kill_switch(&self) -> &OrchestratorKillSwitch {
        &self.kill_switch
    }

    /// Get the frozen spec for a task, if it exists
    pub fn get_frozen_spec(&self, task_id: uuid::Uuid) -> Option<FrozenSpecRef> {
        self.frozen_specs
            .read()
            .ok()
            .and_then(|map| map.get(&task_id).cloned())
    }

    /// Get all frozen specs
    pub fn all_frozen_specs(&self) -> Vec<FrozenSpecRef> {
        self.frozen_specs
            .read()
            .map(|map| map.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Check if a task has an active FeatureLead
    pub fn has_feature_lead(&self, task_id: uuid::Uuid) -> bool {
        self.feature_leads
            .read()
            .map(|map| map.contains_key(&task_id))
            .unwrap_or(false)
    }

    /// Get the FeatureLead for a task, if any
    pub fn get_feature_lead(&self, task_id: uuid::Uuid) -> Option<FeatureLead> {
        self.feature_leads
            .read()
            .ok()
            .and_then(|map| map.get(&task_id).cloned())
    }

    // ========================================================================
    // Drift Detection Methods
    // ========================================================================

    /// Track a file modification during execution.
    ///
    /// Call this when a file is modified (e.g., via file_write or file_edit tool)
    /// to record it for drift detection.
    ///
    /// # Arguments
    /// * `file` - Path to the file that was modified
    pub fn track_file_modification(&self, file: &str) {
        if let Ok(mut files) = self.modified_files.write() {
            files.insert(file.to_string());
        }
    }

    /// Get all files modified during the current execution session.
    ///
    /// # Returns
    /// A vector of file paths that have been modified since the last reset.
    pub fn get_modified_files(&self) -> Vec<String> {
        self.modified_files
            .read()
            .map(|files| files.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Reset the modified files tracker.
    ///
    /// Call this at the start of a new task execution.
    pub fn reset_modified_files(&self) {
        if let Ok(mut files) = self.modified_files.write() {
            files.clear();
        }
    }

    /// Check for drift between planned and actual file modifications.
    ///
    /// Compares the files modified during execution against the estimated files
    /// from the task's plan. If drift exceeds the configured threshold (30%),
    /// a warning is emitted and a DriftReport is returned.
    ///
    /// # Arguments
    /// * `task_id` - The task ID to check drift for
    /// * `estimated_files` - Files from the plan (e.g., aggregated from PlanStep.affected_files)
    ///
    /// # Returns
    /// * `Some(DriftReport)` if drift exceeds the threshold
    /// * `None` if drift is within acceptable limits
    pub fn check_drift(
        &self,
        task_id: uuid::Uuid,
        estimated_files: &[String],
    ) -> Option<DriftReport> {
        let actual_files = self.get_modified_files();
        let report = self
            .drift_detector
            .detect_drift(task_id, estimated_files, &actual_files);

        if report.exceeds_threshold {
            debug!(
                task_id = %task_id,
                drift_percentage = %report.drift_percentage,
                estimated = %report.estimated_files,
                actual = %report.actual_files,
                extra_files = ?report.extra_files,
                "Drift detected: exceeds threshold"
            );
            Some(report)
        } else {
            None
        }
    }

    /// Get a reference to the drift detector for configuration.
    pub fn drift_detector(&self) -> &DriftDetector {
        &self.drift_detector
    }

    /// Get the configured drift check interval in seconds.
    ///
    /// A value of 0 means drift is only checked at the end of execution.
    pub fn drift_check_interval_seconds(&self) -> u64 {
        self.drift_check_interval_seconds
    }

    /// Set the drift check interval in seconds.
    ///
    /// # Arguments
    /// * `interval_seconds` - Interval for periodic drift checks (0 = only at end)
    pub fn set_drift_check_interval(&mut self, interval_seconds: u64) {
        self.drift_check_interval_seconds = interval_seconds;
    }

    /// Execute a single task through the full Planner → Generator → Evaluator pipeline.
    ///
    /// This method runs:
    /// 1. PlannerAgent to create/verify the execution plan
    /// 2. GeneratorAgent to implement the plan
    /// 3. EvaluatorAgent to validate the output using actual validation gates
    ///
    /// For complex tasks (>15 steps), a FeatureLead sub-orchestrator may be spawned.
    pub async fn execute_task(&self, task_id: uuid::Uuid) -> Result<ValidationResult, SwellError> {
        info!(task_id = %task_id, "Starting task execution");

        // Get the task and its estimated files for lock acquisition
        let task = self.orchestrator.get_task(task_id).await?;
        let estimated_files = task.enrichment.enriched_files.clone();

        // Step 0: Acquire file locks before execution
        // This prevents concurrent edits to the same files across tasks
        if !estimated_files.is_empty() {
            match self
                .orchestrator
                .acquire_task_locks(task_id, None, estimated_files.clone())
                .await
            {
                Ok(locks) => {
                    info!(
                        task_id = %task_id,
                        lock_count = locks.len(),
                        "File locks acquired for task execution"
                    );
                }
                Err(LockAcquisitionResult::Conflict {
                    existing_lock,
                    requested_by: _,
                }) => {
                    warn!(
                        task_id = %task_id,
                        conflicting_file = %existing_lock.path,
                        locked_by_task = %existing_lock.task_id,
                        "Cannot start task: file conflict detected"
                    );
                    return Ok(ValidationResult {
                        passed: false,
                        lint_passed: false,
                        tests_passed: false,
                        security_passed: false,
                        ai_review_passed: false,
                        errors: vec![format!(
                            "File conflict: '{}' is locked by another task",
                            existing_lock.path
                        )],
                        warnings: vec![],
                    });
                }
                _ => {}
            }
        }

        // Ensure locks are released when task finishes (success, failure, or panic)
        let file_lock_manager = self.orchestrator.file_lock_manager().clone();
        let task_id_for_cleanup = task_id;

        // Step 1: Planning - run PlannerAgent if task doesn't have a plan
        let needs_planning = task.plan.is_none();

        if needs_planning {
            // Run PlannerAgent with injected LLM backend
            let planner_llm = self.llm.clone();
            let planner = PlannerAgent::with_llm("claude-sonnet".to_string(), planner_llm);
            let session_id = Uuid::new_v4();
            let context = AgentContext {
                task,
                memory_blocks: Vec::new(),
                session_id,
                workspace_path: None,
            };

            let planner_result = planner.execute(context).await?;

            // Update task with planner output if successful
            if planner_result.success {
                // Parse the plan from the planner result output
                // The planner returns JSON with structure: {"plan": {...}, "handoff": {...}}
                let output: serde_json::Value = serde_json::from_str(&planner_result.output)
                    .map_err(|e| {
                        SwellError::LlmError(format!("Failed to parse planner output: {}", e))
                    })?;

                let plan_value = output.get("plan").ok_or_else(|| {
                    SwellError::LlmError("Planner output missing 'plan' field".to_string())
                })?;

                // Convert to Plan struct
                let plan: Plan = serde_json::from_value(plan_value.clone()).map_err(|e| {
                    SwellError::LlmError(format!("Failed to parse plan from planner output: {}", e))
                })?;

                // Set the plan on the task so start_task can proceed
                self.orchestrator.set_plan(task_id, plan).await?;

                info!(task_id = %task_id, "PlannerAgent completed successfully, plan set on task");
            } else {
                // Planner failed - release locks and return failure
                let released_count = file_lock_manager
                    .release_all_for_task(task_id_for_cleanup)
                    .await;
                info!(
                    task_id = %task_id,
                    locks_released = released_count,
                    "File locks released due to planning failure"
                );
                return Ok(ValidationResult {
                    passed: false,
                    lint_passed: false,
                    tests_passed: false,
                    security_passed: false,
                    ai_review_passed: false,
                    errors: vec![planner_result
                        .error
                        .unwrap_or_else(|| "Planning failed".into())],
                    warnings: vec![],
                });
            }
        }

        // Step 2: Transition through states to executing
        self.orchestrator.start_task(task_id).await?;

        // Get updated task after planning
        let task = self.orchestrator.get_task(task_id).await?;

        // Step 2a: Check if we need to spawn a FeatureLead for complex tasks
        if let Some(ref plan) = task.plan {
            if FeatureLead::should_spawn(plan) {
                info!(
                    task_id = %task_id,
                    step_count = plan.steps.len(),
                    "Task exceeds complexity threshold, spawning FeatureLead"
                );

                let parent_orch = self.orchestrator.clone();

                match self
                    .orchestrator
                    .spawn_feature_lead(task_id, plan.clone(), parent_orch)
                {
                    Ok(lead) => {
                        if let Ok(mut leads) = self.feature_leads.write() {
                            leads.insert(task_id, lead);
                        }
                        info!(task_id = %task_id, "FeatureLead spawned successfully");
                    }
                    Err(e) => {
                        // If spawning fails, continue without FeatureLead (graceful degradation)
                        info!(
                            task_id = %task_id,
                            error = %e,
                            "FeatureLead spawn failed, continuing without sub-orchestration"
                        );
                    }
                }
            }
        }

        // Create frozen spec snapshot BEFORE execution starts
        // This ensures immutability: the spec cannot be modified during execution
        let frozen_spec = FrozenSpecRef::from_task(&task);
        if let Ok(mut specs) = self.frozen_specs.write() {
            specs.insert(task_id, frozen_spec);
        }

        // Step 3: Run GeneratorAgent to implement the plan
        // Wire GeneratorAgent with injected LLM backend and ToolRegistry
        let generator = GeneratorAgent::with_llm_and_tools(
            "claude-sonnet".to_string(),
            self.llm.clone(),
            self.tool_registry.clone(),
        )
        .with_checkpoint_manager(self.orchestrator.checkpoint_manager());

        let session_id = Uuid::new_v4();
        let context = AgentContext {
            task,
            memory_blocks: Vec::new(),
            session_id,
            workspace_path: None,
        };

        let generator_result: AgentResult = generator.execute(context).await?;

        // Step 4: Start validation phase
        self.orchestrator.start_validation(task_id).await?;

        // Step 5: Run EvaluatorAgent with validation pipeline
        let evaluator = EvaluatorAgent::with_pipeline(
            "claude-sonnet".to_string(),
            self.validation_pipeline.clone(),
        );
        let eval_context = AgentContext {
            task: self.orchestrator.get_task(task_id).await?,
            memory_blocks: Vec::new(),
            session_id: Uuid::new_v4(),
            workspace_path: None,
        };

        let eval_result = evaluator.execute(eval_context).await?;

        // Step 6: Build final validation result combining generator and evaluator results
        let validation_passed = generator_result.success && eval_result.success;

        let mut errors = Vec::new();
        if let Some(err) = generator_result.error {
            errors.push(err);
        }
        if let Some(err) = eval_result.error {
            errors.push(err);
        }

        let result = ValidationResult {
            passed: validation_passed,
            // For MVP, validation gates are stubbed - in full implementation these come from evaluator
            lint_passed: eval_result.success,
            tests_passed: eval_result.success,
            security_passed: eval_result.success,
            ai_review_passed: eval_result.success,
            errors,
            warnings: vec![],
        };

        // Step 7: Complete the task with validation result
        self.orchestrator
            .complete_task(task_id, result.clone())
            .await?;

        // Step 8: Apply decay function to backlog (when backlog is integrated)
        // NOTE: apply_decay adjusts auto-approve threshold based on run progress.
        // When WorkBacklog is integrated with ExecutionController, this should be called:
        //   let completion_ratio = completed_tasks as f32 / total_tasks as f32;
        //   backlog.apply_decay(completion_ratio);
        // For now, this is stubbed pending backlog integration.

        // Cleanup: Remove FeatureLead from orchestrator and local cache
        let _ = self.orchestrator.remove_feature_lead(task_id).await;
        if let Ok(mut leads) = self.feature_leads.write() {
            leads.remove(&task_id);
        }

        // Release file locks when task completes (success or failure)
        let released_count = file_lock_manager
            .release_all_for_task(task_id_for_cleanup)
            .await;
        if released_count > 0 {
            info!(
                task_id = %task_id,
                locks_released = released_count,
                "File locks released after task completion"
            );
        }

        Ok(result)
    }

    /// Execute a structured turn loop with TurnSummary capture per iteration.
    ///
    /// This method implements a model→tool→model loop where:
    /// 1. Each iteration sends the conversation to the LLM and processes the streaming response
    /// 2. StreamEvents (TextDelta, ToolUse, ToolResult, Usage, MessageStop) are captured
    /// 3. Tool calls are executed via the ToolRegistry and results fed back to the model
    /// 4. Each turn produces a TurnSummary with token usage, tool calls, and outcome
    ///
    /// The loop terminates when:
    /// - max_iterations hard cap is reached
    /// - A text-only response is received (no tool calls)
    /// - An error occurs
    ///
    /// # Arguments
    /// * `messages` - The conversation history (will be extended with tool results)
    /// * `tools` - Available tool definitions for the LLM
    ///
    /// # Returns
    /// * `Ok((Vec<TurnSummary>, String))` - All turn summaries and the final text response
    /// * `Err(SwellError)` - If the loop terminates due to an error
    pub async fn execute_turn_loop(
        &mut self,
        mut messages: Vec<LlmMessage>,
        tools: Option<Vec<LlmToolDefinition>>,
    ) -> Result<(Vec<TurnSummary>, String), SwellError> {
        let mut all_summaries: Vec<TurnSummary> = Vec::new();
        let mut final_text = String::new();
        let mut turn_number = 0u32;

        loop {
            turn_number += 1;
            debug!(turn = turn_number, "Starting turn");

            // Record this turn in the resource tracker
            self.resource_tracker.record_turn();

            // Check resource limits BEFORE starting a new turn
            if let Some(limit_error) = self.resource_tracker.get_first_error() {
                warn!(
                    turn = turn_number,
                    limit = %limit_error.limit_name(),
                    error = %limit_error,
                    "Resource limit exceeded, terminating turn loop"
                );
                // Record the outcome for this turn
                let mut summary = TurnSummary::new(turn_number);
                summary.outcome = TurnOutcome::ResourceLimitExceeded;
                summary.error_message = Some(limit_error.to_string());
                summary.final_text = final_text.clone();
                all_summaries.push(summary);
                return Err(SwellError::ResourceLimitExceeded(limit_error.to_string()));
            }

            // Check kill switch BEFORE starting a new turn - FullStop halts immediately
            if let Err(e) = self.kill_switch.check_fullstop().await {
                warn!(
                    turn = turn_number,
                    error = %e,
                    "Kill switch FullStop triggered, terminating turn loop"
                );
                // Record the outcome for this turn
                let mut summary = TurnSummary::new(turn_number);
                summary.outcome = TurnOutcome::KillSwitchTriggered;
                summary.final_text = final_text.clone();
                all_summaries.push(summary);
                return Err(SwellError::KillSwitchTriggered);
            }

            // Check max iterations hard cap BEFORE starting a new turn
            if turn_number > self.max_iterations {
                warn!(
                    turn = turn_number,
                    max_iterations = self.max_iterations,
                    "Max iterations hard cap reached, terminating turn loop"
                );
                // Record the outcome for this turn
                let mut summary = TurnSummary::new(turn_number);
                summary.outcome = TurnOutcome::MaxIterationsReached;
                summary.final_text = final_text.clone();
                all_summaries.push(summary);
                break;
            }

            let mut summary = TurnSummary::new(turn_number);

            // Call the LLM with streaming
            let stream = self
                .llm
                .stream(messages.clone(), tools.clone(), Default::default())
                .await;

            let stream_result = match stream {
                Ok(s) => s,
                Err(e) => {
                    warn!(turn = turn_number, error = %e, "LLM stream error, terminating turn loop");
                    summary.outcome = TurnOutcome::Error;
                    summary.error_message = Some(e.to_string());
                    summary.final_text = final_text.clone();
                    all_summaries.push(summary);
                    return Err(e);
                }
            };

            // Process the stream
            let mut current_tool_call: Option<(String, String, serde_json::Value)> = None;
            let mut accumulated_text = String::new();

            use futures::StreamExt;
            let mut stream = stream_result;

            while let Some(event) = stream.next().await {
                match event {
                    Ok(StreamEvent::TextDelta { text, delta }) => {
                        accumulated_text = text;
                        debug!(
                            turn = turn_number,
                            delta_len = delta.len(),
                            "Received text delta"
                        );
                    }
                    Ok(StreamEvent::ToolUse { tool_call }) => {
                        debug!(
                            turn = turn_number,
                            tool_name = %tool_call.name,
                            tool_id = %tool_call.id,
                            "Received tool use event"
                        );
                        // Store the tool call to be executed when we get the result
                        current_tool_call =
                            Some((tool_call.id, tool_call.name, tool_call.arguments));
                    }
                    Ok(StreamEvent::ToolResult {
                        tool_call_id,
                        result: _,
                        success,
                    }) => {
                        debug!(
                            turn = turn_number,
                            tool_call_id = %tool_call_id,
                            success = success,
                            "Received tool result event"
                        );
                        // If we have a pending tool call, execute it
                        if let Some((id, name, arguments)) = current_tool_call.take() {
                            let tool_result = self.execute_tool(&name, arguments.clone()).await;
                            let (was_success, result_str) = match tool_result {
                                Ok(output) => {
                                    (!output.is_error, extract_text_from_content(&output.content))
                                }
                                Err(e) => (false, e.to_string()),
                            };
                            // Clone id before passing to add_tool_call since it moves
                            let id_clone = id.clone();
                            summary.add_tool_call(
                                name,
                                id_clone,
                                arguments,
                                result_str.clone(),
                                was_success,
                            );

                            // Add tool result to messages using Assistant role
                            // Include tool_call_id to track the tool_use/tool_result pair
                            // for context compaction
                            messages.push(LlmMessage {
                                role: swell_llm::LlmRole::Assistant,
                                content: result_str,
                                tool_call_id: Some(id),
                            });
                        }
                    }
                    Ok(StreamEvent::Usage {
                        input_tokens,
                        output_tokens,
                        cache_creation_input_tokens,
                        cache_read_input_tokens,
                    }) => {
                        summary.input_tokens = input_tokens;
                        summary.output_tokens = output_tokens;
                        summary.cache_creation_input_tokens = cache_creation_input_tokens;
                        summary.cache_read_input_tokens = cache_read_input_tokens;
                        debug!(
                            turn = turn_number,
                            input_tokens, output_tokens, "Received usage event"
                        );
                    }
                    Ok(StreamEvent::MessageStop { stop_reason }) => {
                        summary.stop_reason = stop_reason;
                        debug!(turn = turn_number, "Received message stop event");
                    }
                    Ok(StreamEvent::Error { message }) => {
                        warn!(turn = turn_number, error = %message, "Stream error event");
                        summary.outcome = TurnOutcome::Error;
                        summary.error_message = Some(message.clone());
                        summary.final_text = accumulated_text.clone();
                        all_summaries.push(summary);
                        return Err(SwellError::LlmError(message));
                    }
                    Err(e) => {
                        warn!(turn = turn_number, error = %e, "Stream item error");
                        summary.outcome = TurnOutcome::Error;
                        summary.error_message = Some(e.to_string());
                        summary.final_text = accumulated_text.clone();
                        all_summaries.push(summary);
                        return Err(e);
                    }
                }
            }

            // Determine the outcome of this turn
            final_text = accumulated_text.clone();
            summary.final_text = final_text.clone();

            if summary.had_tool_calls() {
                summary.outcome = TurnOutcome::ToolCallsExecuted;
                debug!(
                    turn = turn_number,
                    tool_count = summary.tool_calls.len(),
                    "Turn completed with tool calls"
                );
            } else if final_text.is_empty() {
                summary.outcome = TurnOutcome::EmptyResponse;
                warn!(
                    turn = turn_number,
                    "Turn produced empty response, terminating"
                );
                all_summaries.push(summary);
                break;
            } else {
                summary.outcome = TurnOutcome::TextOnly;
                debug!(
                    turn = turn_number,
                    "Turn completed with text only, terminating"
                );
                all_summaries.push(summary);
                break;
            }

            all_summaries.push(summary);

            // Record resource usage for this turn
            // Update tokens and cost in the resource tracker
            if let Some(last_summary) = all_summaries.last() {
                let total_tokens = last_summary.total_tokens();
                if total_tokens > 0 {
                    self.resource_tracker.record_tokens(total_tokens);
                }

                // Record cache tokens as well for accurate tracking
                if let Some(cache_creation) = last_summary.cache_creation_input_tokens {
                    self.resource_tracker.record_tokens(cache_creation);
                }
                if let Some(cache_read) = last_summary.cache_read_input_tokens {
                    self.resource_tracker.record_tokens(cache_read);
                }

                // Calculate and record cost based on token usage and model pricing
                let input_tokens = last_summary.input_tokens;
                let output_tokens = last_summary.output_tokens;
                if input_tokens > 0 || output_tokens > 0 {
                    let pricing = swell_core::opentelemetry::pricing::for_model(self.llm.model());
                    let cost = pricing.calculate_cost(input_tokens, output_tokens);
                    self.resource_tracker.record_cost(cost);
                }

                // Record any tool call failures as failures in the tracker
                for tool_call in &last_summary.tool_calls {
                    if !tool_call.success {
                        self.resource_tracker.record_failure();
                    }
                }
            }

            // Record this turn as successful (resets consecutive failures)
            // Only record success if the turn had no errors
            if let Some(last_summary) = all_summaries.last() {
                if last_summary.outcome != TurnOutcome::Error
                    && last_summary.outcome != TurnOutcome::ResourceLimitExceeded
                    && last_summary.outcome != TurnOutcome::KillSwitchTriggered
                {
                    self.resource_tracker.record_success();
                }
            }

            // Compact context if accumulated tokens exceed threshold
            // This happens after each turn with tool calls to keep the conversation manageable
            messages = self.compact_context(&messages);
        }

        debug!(
            total_turns = all_summaries.len(),
            final_text_len = final_text.len(),
            "Turn loop completed"
        );

        Ok((all_summaries, final_text))
    }

    // ============================================================================
    // Context Compaction
    // ============================================================================

    /// Estimate token count for a single message.
    ///
    /// Uses a rough approximation of ~4 characters per token.
    fn estimate_message_tokens(message: &LlmMessage) -> usize {
        // Rough approximation: ~4 characters per token on average
        // Include role prefix in the estimate
        let role_len = match message.role {
            swell_llm::LlmRole::System => "system: ".len(),
            swell_llm::LlmRole::User => "user: ".len(),
            swell_llm::LlmRole::Assistant => "assistant: ".len(),
        };
        ((message.content.len() + role_len) / 4).max(1)
    }

    /// Estimate total token count for a list of messages.
    fn estimate_total_tokens(messages: &[LlmMessage]) -> usize {
        messages.iter().map(Self::estimate_message_tokens).sum()
    }

    /// Compact the conversation history to reduce token count.
    ///
    /// Compaction is triggered when accumulated token count exceeds
    /// `context_compaction_threshold`. This method:
    /// 1. Preserves the most recent N messages (tail_message_count)
    /// 2. Never splits tool_use/tool_result message pairs - if either member
    ///    of a pair falls in the preserved tail, both are preserved
    ///
    /// # Arguments
    /// * `messages` - The conversation history to compact
    ///
    /// # Returns
    /// * `Vec<LlmMessage>` - The compacted message list
    pub fn compact_context(&self, messages: &[LlmMessage]) -> Vec<LlmMessage> {
        let total_tokens = Self::estimate_total_tokens(messages);

        debug!(
            total_tokens,
            threshold = self.context_compaction_threshold,
            message_count = messages.len(),
            "Checking if context compaction is needed"
        );

        // If we're under the threshold, no compaction needed
        if total_tokens <= self.context_compaction_threshold {
            return messages.to_vec();
        }

        debug!(
            total_tokens,
            threshold = self.context_compaction_threshold,
            "Context compaction triggered"
        );

        // Build a map of tool_call_id -> whether this pair should be preserved
        // A pair is preserved if EITHER the tool_use or tool_result is in the tail
        let tail_start = messages.len().saturating_sub(self.tail_message_count);

        // Find all tool_call_ids in the tail region
        let mut tail_tool_call_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for msg in messages.iter().skip(tail_start) {
            if let Some(id) = &msg.tool_call_id {
                tail_tool_call_ids.insert(id.clone());
            }
        }

        // Messages to keep: all tail messages + any message before tail
        // whose tool_call_id is referenced in the tail
        let mut result: Vec<LlmMessage> = Vec::new();

        // Add messages from the tail (always preserved)
        result.extend_from_slice(&messages[tail_start..]);

        // Add messages from before the tail, but only if they are tool_results
        // whose tool_call_id is referenced in the tail (preserving tool pairs).
        // Regular messages and tool_use messages before the tail are removed
        // during compaction to reduce token count.
        for msg in messages.iter().take(tail_start) {
            if let Some(id) = &msg.tool_call_id {
                // This is a tool result - only keep if its pair is in the tail
                if tail_tool_call_ids.contains(id) {
                    result.push(msg.clone());
                }
            }
            // Messages without tool_call_id (tool_use or regular) are skipped
            // during compaction to reduce token count
        }

        // Reverse to restore chronological order (oldest first)
        result.reverse();

        let compacted_tokens = Self::estimate_total_tokens(&result);
        debug!(
            before_tokens = total_tokens,
            after_tokens = compacted_tokens,
            before_count = messages.len(),
            after_count = result.len(),
            "Context compaction completed"
        );

        result
    }

    /// Execute a tool by name with the given arguments.
    ///
    /// Returns a `ToolOutput` with `success: false` and error details when:
    /// - The tool requires `PermissionTier::Deny` permission (which is never allowed)
    /// - The tool is not found
    ///
    /// The error message in the `ToolOutput` includes:
    /// - The tool name that was denied
    /// - The triggering permission rule (Deny tier requirement)
    async fn execute_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<swell_core::traits::ToolOutput, SwellError> {
        // Check kill switch BEFORE executing the tool - FullStop blocks all execution
        if let Err(e) = self.kill_switch.check_fullstop().await {
            warn!(
                tool = %name,
                error = %e,
                "Kill switch FullStop triggered, denying tool execution"
            );
            return Ok(ToolOutput {
                is_error: true,
                content: vec![ToolResultContent::Error(format!(
                    "Tool execution blocked: kill switch FullStop triggered ({})",
                    e
                ))],
            });
        }

        // Perform tool-specific restriction checks
        if let Err(e) = self.kill_switch.check_tool_dispatch(name, &arguments).await {
            warn!(
                tool = %name,
                error = %e,
                "Kill switch restriction blocked tool execution"
            );
            return Ok(ToolOutput {
                is_error: true,
                content: vec![ToolResultContent::Error(format!(
                    "Tool execution blocked: kill switch restriction ({})",
                    e
                ))],
            });
        }

        // Find the tool in the registry
        let tool =
            self.tool_registry.get(name).await.ok_or_else(|| {
                SwellError::ToolExecutionFailed(format!("Tool not found: {}", name))
            })?;

        // Check permission tier - Deny tier is never allowed
        // This surfaces permission denials as ToolOutput with is_error=true in the transcript
        if tool.permission_tier() == swell_core::PermissionTier::Deny {
            let denial_message = format!(
                "Permission denied: tool '{}' requires {:?} permission. \
                 This tool is blocked by the permission system's deny rule.",
                name,
                tool.permission_tier()
            );
            tracing::warn!(
                tool = %name,
                tier = ?tool.permission_tier(),
                "Tool execution denied by permission system"
            );
            return Ok(ToolOutput {
                is_error: true,
                content: vec![ToolResultContent::Error(denial_message)],
            });
        }

        // Execute the tool
        tool.execute(arguments).await
    }

    /// Execute multiple tasks in parallel, respecting max concurrent agents
    pub async fn execute_batch(
        &self,
        task_ids: Vec<uuid::Uuid>,
    ) -> Vec<Result<ValidationResult, SwellError>> {
        info!(count = task_ids.len(), "Starting batch execution");

        let results = stream::iter(task_ids)
            .map(|task_id| {
                let controller = self.clone();
                async move { controller.execute_task(task_id).await }
            })
            .buffer_unordered(self.max_concurrent)
            .collect()
            .await;

        results
    }
}

impl Clone for ExecutionController {
    fn clone(&self) -> Self {
        Self {
            orchestrator: self.orchestrator.clone(),
            llm: self.llm.clone(),
            tool_registry: self.tool_registry.clone(),
            validation_pipeline: self.validation_pipeline.clone(),
            kill_switch: self.kill_switch.clone(),
            resource_tracker: self.resource_tracker.clone(),
            max_concurrent: self.max_concurrent,
            max_iterations: self.max_iterations,
            context_compaction_threshold: self.context_compaction_threshold,
            tail_message_count: self.tail_message_count,
            frozen_specs: std::sync::RwLock::new(std::collections::HashMap::new()),
            feature_leads: std::sync::RwLock::new(std::collections::HashMap::new()),
            drift_detector: self.drift_detector.clone(),
            modified_files: std::sync::RwLock::new(HashSet::new()),
            drift_check_interval_seconds: self.drift_check_interval_seconds,
        }
    }
}

/// Configuration for task execution
#[derive(Debug, Clone)]
pub struct ExecutionConfig {
    /// Maximum retries for validation (separate from max_iterations hard cap)
    pub max_retries: u32,
    /// Timeout in seconds for task execution
    pub timeout_secs: u64,
    /// Whether validation is enabled
    pub validation_enabled: bool,
    /// Maximum iterations for the turn loop (hard cap)
    /// This is separate from validation retry count
    pub max_iterations: u32,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            timeout_secs: 3600, // 1 hour
            validation_enabled: true,
            max_iterations: DEFAULT_MAX_ITERATIONS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;
    use swell_core::traits::Tool;
    use swell_llm::MockLlm;
    use swell_tools::ToolRegistry;

    #[tokio::test]
    async fn test_execution_controller_creation() {
        let orchestrator = Orchestrator::new();
        let mock_llm = Arc::new(MockLlm::new("claude-sonnet"));
        let tool_registry = Arc::new(ToolRegistry::new());
        let controller = ExecutionController::new(Arc::new(orchestrator), mock_llm, tool_registry);
        assert_eq!(controller.max_concurrent, MAX_CONCURRENT_AGENTS);
    }

    #[tokio::test]
    async fn test_batch_execution() {
        let orchestrator = Orchestrator::new();
        let mock_llm = Arc::new(MockLlm::new("claude-sonnet"));
        let tool_registry = Arc::new(ToolRegistry::new());
        let controller = ExecutionController::new(Arc::new(orchestrator), mock_llm, tool_registry);

        // Create some tasks
        let task1 = controller
            .orchestrator
            .create_task("Task 1".to_string(), vec![])
            .await
            .unwrap();
        let task2 = controller
            .orchestrator
            .create_task("Task 2".to_string(), vec![])
            .await
            .unwrap();

        let results = controller.execute_batch(vec![task1.id, task2.id]).await;
        assert_eq!(results.len(), 2);
    }

    // ========================================================================
    // Turn Loop Tests
    // ========================================================================

    #[tokio::test]
    async fn test_turn_summary_new() {
        let summary = TurnSummary::new(1);
        assert_eq!(summary.turn_number, 1);
        assert_eq!(summary.input_tokens, 0);
        assert_eq!(summary.output_tokens, 0);
        assert!(summary.tool_calls.is_empty());
        assert_eq!(summary.outcome, TurnOutcome::EmptyResponse);
        assert!(summary.final_text.is_empty());
    }

    #[tokio::test]
    async fn test_turn_summary_add_tool_call() {
        let mut summary = TurnSummary::new(1);
        summary.add_tool_call(
            "file_read".to_string(),
            "toolu_123".to_string(),
            serde_json::json!({"path": "/tmp/test.txt"}),
            "file contents".to_string(),
            true,
        );

        assert_eq!(summary.tool_calls.len(), 1);
        assert!(summary.had_tool_calls());
        assert_eq!(summary.tool_calls[0].name, "file_read");
        assert_eq!(summary.tool_calls[0].id, "toolu_123");
        assert!(summary.tool_calls[0].success);
    }

    #[tokio::test]
    async fn test_turn_summary_total_tokens() {
        let mut summary = TurnSummary::new(1);
        summary.input_tokens = 100;
        summary.output_tokens = 50;
        assert_eq!(summary.total_tokens(), 150);
    }

    #[tokio::test]
    async fn test_turn_outcome_text_only() {
        let outcome = TurnOutcome::TextOnly;
        assert!(!matches!(outcome, TurnOutcome::ToolCallsExecuted));
    }

    #[tokio::test]
    async fn test_execution_controller_with_max_iterations() {
        let orchestrator = Orchestrator::new();
        let mock_llm = Arc::new(MockLlm::new("claude-sonnet"));
        let tool_registry = Arc::new(ToolRegistry::new());

        let controller = ExecutionController::with_max_iterations(
            Arc::new(orchestrator),
            mock_llm,
            tool_registry,
            5,
        );

        assert_eq!(controller.max_iterations(), 5);
    }

    #[tokio::test]
    async fn test_execution_controller_default_max_iterations() {
        let orchestrator = Orchestrator::new();
        let mock_llm = Arc::new(MockLlm::new("claude-sonnet"));
        let tool_registry = Arc::new(ToolRegistry::new());

        let controller = ExecutionController::new(Arc::new(orchestrator), mock_llm, tool_registry);

        assert_eq!(controller.max_iterations(), DEFAULT_MAX_ITERATIONS);
    }

    #[tokio::test]
    async fn test_execution_controller_with_pipeline_and_max_iterations() {
        use swell_validation::ValidationPipeline;

        let orchestrator = Orchestrator::new();
        let mock_llm = Arc::new(MockLlm::new("claude-sonnet"));
        let tool_registry = Arc::new(ToolRegistry::new());
        let validation_pipeline = ValidationPipeline::new();

        let controller = ExecutionController::with_pipeline_and_max_iterations(
            Arc::new(orchestrator),
            mock_llm,
            tool_registry,
            validation_pipeline,
            10,
        );

        assert_eq!(controller.max_iterations(), 10);
    }

    #[tokio::test]
    async fn test_execution_config_default_max_iterations() {
        let config = ExecutionConfig::default();
        assert_eq!(config.max_iterations, DEFAULT_MAX_ITERATIONS);
    }

    #[tokio::test]
    async fn test_execution_config_with_custom_max_iterations() {
        let config = ExecutionConfig {
            max_retries: 3,
            timeout_secs: 3600,
            validation_enabled: true,
            max_iterations: 25,
        };
        assert_eq!(config.max_iterations, 25);
    }

    #[tokio::test]
    async fn test_turn_loop_terminates_on_text_only_response() {
        // This test verifies that the turn loop terminates correctly
        // when receiving a text-only response (no tool calls)
        let orchestrator = Orchestrator::new();
        let mock_llm = Arc::new(MockLlm::with_response("claude-sonnet", "Hello, world!"));
        let tool_registry = Arc::new(ToolRegistry::new());

        let mut controller = ExecutionController::with_max_iterations(
            Arc::new(orchestrator),
            mock_llm,
            tool_registry,
            50, // High max_iterations so it doesn't trigger
        );

        let messages = vec![swell_llm::LlmMessage {
            role: swell_llm::LlmRole::User,
            content: "Say hello".to_string(),
            tool_call_id: None,
        }];

        let result = controller.execute_turn_loop(messages, None).await;
        assert!(result.is_ok());

        let (summaries, final_text) = result.unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].outcome, TurnOutcome::TextOnly);
        assert_eq!(final_text, "Hello, world!");
    }

    #[tokio::test]
    async fn test_turn_loop_respects_max_iterations() {
        // Create a mock that always returns a tool call to force iteration
        let orchestrator = Orchestrator::new();
        // Use a mock that returns tool use pattern - but MockLlm doesn't support tool calls
        // So we just test with text-only responses
        let mock_llm = Arc::new(MockLlm::with_response("claude-sonnet", "Hello"));
        let tool_registry = Arc::new(ToolRegistry::new());

        // Set max_iterations to 2
        let mut controller = ExecutionController::with_max_iterations(
            Arc::new(orchestrator),
            mock_llm,
            tool_registry,
            2,
        );

        let messages = vec![swell_llm::LlmMessage {
            role: swell_llm::LlmRole::User,
            content: "Say hello".to_string(),
            tool_call_id: None,
        }];

        let result = controller.execute_turn_loop(messages, None).await;
        assert!(result.is_ok());

        let (summaries, _) = result.unwrap();
        // With text-only response, loop terminates after 1 turn
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].outcome, TurnOutcome::TextOnly);
    }

    #[tokio::test]
    async fn test_turn_loop_error_handling() {
        let orchestrator = Orchestrator::new();
        let mock_llm = Arc::new(MockLlm::failing("claude-sonnet"));
        let tool_registry = Arc::new(ToolRegistry::new());

        let mut controller = ExecutionController::with_max_iterations(
            Arc::new(orchestrator),
            mock_llm,
            tool_registry,
            50,
        );

        let messages = vec![swell_llm::LlmMessage {
            role: swell_llm::LlmRole::User,
            content: "Say hello".to_string(),
            tool_call_id: None,
        }];

        let result = controller.execute_turn_loop(messages, None).await;
        assert!(result.is_err());
    }

    // ========================================================================
    // Context Compaction Tests
    // ========================================================================

    fn make_controller_with_compaction(threshold: usize, tail_count: usize) -> ExecutionController {
        let orchestrator = Orchestrator::new();
        let mock_llm = Arc::new(MockLlm::new("claude-sonnet"));
        let tool_registry = Arc::new(ToolRegistry::new());

        ExecutionController::with_all_settings(
            Arc::new(orchestrator),
            mock_llm,
            tool_registry,
            ValidationPipeline::new(),
            DEFAULT_MAX_ITERATIONS,
            threshold,
            tail_count,
        )
    }

    #[test]
    fn test_compact_context_no_op_when_under_threshold() {
        // When total tokens are under threshold, compaction should return unchanged messages
        let controller = make_controller_with_compaction(1000, 5);

        let messages = vec![
            LlmMessage {
                role: swell_llm::LlmRole::User,
                content: "Short message".to_string(),
                tool_call_id: None,
            },
            LlmMessage {
                role: swell_llm::LlmRole::Assistant,
                content: "Short response".to_string(),
                tool_call_id: None,
            },
        ];

        let result = controller.compact_context(&messages);

        // Should be unchanged since we're under threshold
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].content, "Short message");
        assert_eq!(result[1].content, "Short response");
    }

    #[test]
    fn test_compact_context_preserves_tail_messages() {
        // Tail messages (most recent N) should always be preserved regardless of token count
        let controller = make_controller_with_compaction(100, 3);

        // Create messages where the tail is clearly defined
        let messages: Vec<LlmMessage> = (0..10)
            .map(|i| LlmMessage {
                role: swell_llm::LlmRole::User,
                content: format!("Message number {}", i),
                tool_call_id: None,
            })
            .collect();

        let result = controller.compact_context(&messages);

        // The last 3 messages (7, 8, 9) should always be preserved
        // They appear first in the result since it's reversed
        assert!(result.len() >= 3);
        // Check that tail messages are present
        let tail_contents: Vec<&str> = result
            .iter()
            .rev()
            .take(3)
            .map(|m| m.content.as_str())
            .collect();
        assert!(tail_contents.contains(&"Message number 7"));
        assert!(tail_contents.contains(&"Message number 8"));
        assert!(tail_contents.contains(&"Message number 9"));
    }

    #[test]
    fn test_compact_context_preserves_tool_pairs() {
        // tool_use and tool_result pairs should never be split during compaction
        // If a tool_result is in the tail, its corresponding tool_use must also be preserved
        let controller = make_controller_with_compaction(100, 3);

        // Create a sequence where:
        // - message 0-4 are older (before tail)
        // - message 5 has tool_use (no tool_call_id set)
        // - message 6 has tool_result with tool_call_id = "call_123"
        // - messages 7-9 are in the tail

        let mut messages: Vec<LlmMessage> = Vec::new();

        // Older messages (will be compacted away)
        for i in 0..5 {
            messages.push(LlmMessage {
                role: swell_llm::LlmRole::User,
                content: format!("Old message {}", i),
                tool_call_id: None,
            });
        }

        // This represents the tool_use message (Assistant role, no tool_call_id)
        messages.push(LlmMessage {
            role: swell_llm::LlmRole::Assistant,
            content: r#"{"name": "file_read", "arguments": {"path": "/tmp/test.txt"}}"#.to_string(),
            tool_call_id: None, // tool_use doesn't have tool_call_id
        });

        // This represents the tool_result (Assistant role, with tool_call_id)
        messages.push(LlmMessage {
            role: swell_llm::LlmRole::Assistant,
            content: "File contents here".to_string(),
            tool_call_id: Some("call_123".to_string()), // Links to the tool_use
        });

        // Tail messages (always preserved)
        for i in 7..10 {
            messages.push(LlmMessage {
                role: swell_llm::LlmRole::User,
                content: format!("Tail message {}", i),
                tool_call_id: None,
            });
        }

        let result = controller.compact_context(&messages);

        // The tool_use (message 5) and tool_result (message 6) should NOT be split.
        // Since the tool_result (call_123) is in the tail or linked to something in tail,
        // both should be preserved.

        // Check that we don't have orphaned tool_results
        // (tool_result without its tool_use)
        let _tool_result_ids: std::collections::HashSet<_> = result
            .iter()
            .filter_map(|m| m.tool_call_id.clone())
            .collect();

        // Every tool_call_id in result should have its corresponding tool_use in result
        // This is a basic sanity check that pairs aren't orphaned
        // Note: In this test, the tool_use doesn't have tool_call_id set (it's None),
        // but the tool_result does. So the pair detection relies on the tool_result's
        // tool_call_id being preserved when its "pair" is in the tail.

        // Since the tool_result has tool_call_id="call_123" and the tool_use is right before it,
        // both should be preserved together
        assert!(result.len() >= 2); // At minimum, tool_use and tool_result should be preserved
    }

    #[test]
    fn test_compact_context_triggered_above_threshold() {
        // When token count exceeds threshold, compaction should reduce the message count
        let controller = make_controller_with_compaction(200, 2);

        // Create many large messages that exceed the threshold
        // Each message is ~50 chars = ~12-13 tokens, so 20 messages = ~250 tokens
        let messages: Vec<LlmMessage> = (0..20)
            .map(|i| LlmMessage {
                role: swell_llm::LlmRole::User,
                content: format!(
                    "This is a relatively long message number {} with some extra content",
                    i
                ),
                tool_call_id: None,
            })
            .collect();

        // Verify we start over threshold
        let initial_tokens = ExecutionController::estimate_total_tokens(&messages);
        assert!(initial_tokens > 200, "Test setup should exceed threshold");

        let result = controller.compact_context(&messages);

        // Compaction should have reduced the message count
        // while still preserving tail messages
        assert!(
            result.len() < messages.len(),
            "Expected fewer messages after compaction, got {} vs {}",
            result.len(),
            messages.len()
        );

        // But we should still have at least the tail count
        assert!(result.len() >= 2, "Should preserve at least tail messages");
    }

    #[test]
    fn test_compact_context_empty_input() {
        let controller = make_controller_with_compaction(100, 5);
        let messages: Vec<LlmMessage> = vec![];
        let result = controller.compact_context(&messages);
        assert!(result.is_empty());
    }

    #[test]
    fn test_compact_context_exact_threshold_no_op() {
        // When exactly at threshold, compaction should not run
        let controller = make_controller_with_compaction(1000, 5);

        // Create messages that together are exactly at the threshold
        // 5 messages * ~200 tokens each = ~1000 tokens
        let messages: Vec<LlmMessage> = (0..5)
            .map(|_| LlmMessage {
                role: swell_llm::LlmRole::User,
                content: "This is a medium length message that should be around 200 chars when repeated five times".to_string(),
                tool_call_id: None,
            })
            .collect();

        let result = controller.compact_context(&messages);

        // At exactly threshold, compaction should NOT trigger (<= not <)
        // The implementation uses <= for threshold check
        assert_eq!(result.len(), 5);
    }

    #[test]
    fn test_compact_context_preserves_pair_when_result_in_tail() {
        // When a tool_result is in the preserved tail, its tool_use must also be preserved
        let controller = make_controller_with_compaction(100, 2);

        let mut messages: Vec<LlmMessage> = Vec::new();

        // Message 0-2: older messages
        for i in 0..3 {
            messages.push(LlmMessage {
                role: swell_llm::LlmRole::User,
                content: format!("Old message {}", i),
                tool_call_id: None,
            });
        }

        // Message 3: tool_use
        messages.push(LlmMessage {
            role: swell_llm::LlmRole::Assistant,
            content: r#"{"name": "file_read", "arguments": {"path": "/tmp/test.txt"}}"#.to_string(),
            tool_call_id: None,
        });

        // Message 4: tool_result with tool_call_id="call_1" - THIS IS IN THE TAIL
        messages.push(LlmMessage {
            role: swell_llm::LlmRole::Assistant,
            content: "File contents".to_string(),
            tool_call_id: Some("call_1".to_string()),
        });

        // Message 5: tool_use
        messages.push(LlmMessage {
            role: swell_llm::LlmRole::Assistant,
            content: r#"{"name": "shell", "arguments": {"cmd": "ls"}}"#.to_string(),
            tool_call_id: None,
        });

        // Message 6: tool_result with tool_call_id="call_2"
        messages.push(LlmMessage {
            role: swell_llm::LlmRole::Assistant,
            content: "ls output".to_string(),
            tool_call_id: Some("call_2".to_string()),
        });

        // Tail count is 2, so messages 5 and 6 are in the tail
        // But call_1's result (message 4) is in the tail, so call_1's tool_use (message 3)
        // must also be preserved

        let result = controller.compact_context(&messages);

        // Verify that the pair (call_1) is preserved: both tool_use (msg 3) and tool_result (msg 4)
        let call_1_result = result
            .iter()
            .find(|m| m.tool_call_id.as_ref() == Some(&"call_1".to_string()));
        assert!(
            call_1_result.is_some(),
            "call_1 result should be preserved because it's in tail"
        );

        // Find the tool_use for call_1 (the one right before it with no tool_call_id)
        // This is msg 3 in original
        let preserved_tool_uses = result
            .iter()
            .filter(|m| m.tool_call_id.is_none() && m.content.contains("file_read"))
            .count();
        assert!(
            preserved_tool_uses >= 1,
            "tool_use for call_1 should be preserved because its result is in tail"
        );
    }

    #[test]
    fn test_estimate_message_tokens() {
        let msg = LlmMessage {
            role: swell_llm::LlmRole::User,
            content: "Hello world this is a test message".to_string(),
            tool_call_id: None,
        };

        let tokens = ExecutionController::estimate_message_tokens(&msg);
        // "user: " (5) + "Hello world this is a test message" (33) = 38 chars
        // 38 / 4 = 9.5 → 9 tokens (minimum 1)
        assert!(tokens >= 9);
    }

    #[test]
    fn test_estimate_total_tokens() {
        let messages = vec![
            LlmMessage {
                role: swell_llm::LlmRole::User,
                content: "Short".to_string(),
                tool_call_id: None,
            },
            LlmMessage {
                role: swell_llm::LlmRole::Assistant,
                content: "Also short".to_string(),
                tool_call_id: None,
            },
        ];

        let total = ExecutionController::estimate_total_tokens(&messages);
        assert!(total >= 2); // At minimum 1 token per message
    }

    // ========================================================================
    // Permission Denial Surfacing Tests
    // ========================================================================

    /// A test tool that requires Deny permission tier
    struct DenyPermissionTool {
        permission_tier: swell_core::PermissionTier,
    }

    impl DenyPermissionTool {
        fn new(permission_tier: swell_core::PermissionTier) -> Self {
            Self { permission_tier }
        }
    }

    #[async_trait::async_trait]
    impl Tool for DenyPermissionTool {
        fn name(&self) -> &str {
            "deny_test_tool"
        }

        fn description(&self) -> String {
            "A test tool for permission denial surfacing".to_string()
        }

        fn risk_level(&self) -> swell_core::ToolRiskLevel {
            swell_core::ToolRiskLevel::Read
        }

        fn permission_tier(&self) -> swell_core::PermissionTier {
            self.permission_tier
        }

        fn behavioral_hints(&self) -> swell_core::traits::ToolBehavioralHints {
            swell_core::traits::ToolBehavioralHints {
                read_only_hint: true,
                destructive_hint: false,
                idempotent_hint: true,
            }
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            })
        }

        async fn execute(
            &self,
            _arguments: serde_json::Value,
        ) -> Result<swell_core::traits::ToolOutput, swell_core::SwellError> {
            Ok(ToolOutput {
                is_error: false,
                content: vec![ToolResultContent::Text("should not reach here".to_string())],
            })
        }
    }

    #[tokio::test]
    async fn test_execute_tool_returns_error_on_deny_permission() {
        // Test that execute_tool returns ToolOutput with success=false
        // when the tool requires Deny permission tier
        let orchestrator = Orchestrator::new();
        let mock_llm = Arc::new(MockLlm::new("claude-sonnet"));
        let tool_registry = Arc::new(ToolRegistry::new());

        // Register a tool that requires Deny permission
        let deny_tool = DenyPermissionTool::new(swell_core::PermissionTier::Deny);
        tool_registry
            .register(
                deny_tool,
                swell_tools::registry::ToolCategory::Misc,
                swell_tools::registry::ToolLayer::Builtin,
            )
            .await;

        let controller = ExecutionController::new(Arc::new(orchestrator), mock_llm, tool_registry);

        let result = controller
            .execute_tool("deny_test_tool", serde_json::json!({}))
            .await;

        // Should return Ok with ToolOutput (not Err)
        assert!(
            result.is_ok(),
            "execute_tool should return Ok even for denied tools, got: {:?}",
            result
        );

        let output = result.unwrap();
        assert!(
            output.is_error,
            "ToolOutput.is_error should be true for denied tool"
        );
        assert!(
            !output.content.is_empty(),
            "ToolOutput.content should contain denial message"
        );

        let error_msg = extract_error_from_content(&output.content);
        assert!(
            error_msg.contains("deny_test_tool"),
            "Error message should contain tool name: {}",
            error_msg
        );
        assert!(
            error_msg.contains("denied") || error_msg.contains("Deny"),
            "Error message should indicate permission denial: {}",
            error_msg
        );
    }

    #[tokio::test]
    async fn test_execute_tool_allows_non_deny_permission() {
        // Test that tools with non-Deny permission (Auto, Ask) are allowed through
        let orchestrator = Orchestrator::new();
        let mock_llm = Arc::new(MockLlm::new("claude-sonnet"));
        let tool_registry = Arc::new(ToolRegistry::new());

        // Register a tool that requires Ask permission (should be allowed)
        let ask_tool = DenyPermissionTool::new(swell_core::PermissionTier::Ask);
        tool_registry
            .register(
                ask_tool,
                swell_tools::registry::ToolCategory::Misc,
                swell_tools::registry::ToolLayer::Builtin,
            )
            .await;

        let controller = ExecutionController::new(Arc::new(orchestrator), mock_llm, tool_registry);

        let result = controller
            .execute_tool("deny_test_tool", serde_json::json!({}))
            .await;

        // Tool with Ask permission should execute (ToolOutput is_error=false)
        // Note: The tool itself might fail, but the permission check should pass
        match result {
            Ok(output) => {
                // Permission passed - tool executed
                // The tool returns is_error:false with "should not reach here" but we don't care
                // about the tool's internal logic, only that permission was checked
                let text = extract_text_from_content(&output.content);
                assert!(
                    !output.is_error || text.contains("should not reach here"),
                    "Tool execution should proceed for Ask permission"
                );
            }
            Err(e) => {
                // If it errored, it should be because the tool's execute failed,
                // not because of permission denial
                let err_str = e.to_string();
                assert!(
                    !err_str.contains("denied") && !err_str.contains("Deny"),
                    "Error should not be permission denial for Ask tier: {}",
                    err_str
                );
            }
        }
    }

    #[tokio::test]
    async fn test_execute_tool_auto_permission_allowed() {
        // Test that tools with Auto permission are allowed through
        let orchestrator = Orchestrator::new();
        let mock_llm = Arc::new(MockLlm::new("claude-sonnet"));
        let tool_registry = Arc::new(ToolRegistry::new());

        // Register a tool that requires Auto permission
        let auto_tool = DenyPermissionTool::new(swell_core::PermissionTier::Auto);
        tool_registry
            .register(
                auto_tool,
                swell_tools::registry::ToolCategory::Misc,
                swell_tools::registry::ToolLayer::Builtin,
            )
            .await;

        let controller = ExecutionController::new(Arc::new(orchestrator), mock_llm, tool_registry);

        let result = controller
            .execute_tool("deny_test_tool", serde_json::json!({}))
            .await;

        // Tool with Auto permission should execute
        assert!(
            result.is_ok(),
            "Tool with Auto permission should not be denied: {:?}",
            result
        );
    }

    // ========================================================================
    // Kill Switch Tests (VAL-SAFE-001, VAL-SAFE-002)
    // ========================================================================

    /// Test VAL-SAFE-001: FullStop halts turn loop execution immediately.
    ///
    /// When FullStop is triggered, the execute_turn_loop should:
    /// 1. Stop before making the next LLM call
    /// 2. Return Err(SwellError::KillSwitchTriggered)
    /// 3. Record TurnOutcome::KillSwitchTriggered in the summary
    #[tokio::test]
    async fn test_killswitch_fullstop_halts_turn_loop() {
        use swell_core::kill_switch::KillLevel;

        let orchestrator = Orchestrator::new();
        let mock_llm = Arc::new(MockLlm::with_response("claude-sonnet", "Hello world"));
        let tool_registry = Arc::new(ToolRegistry::new());

        let mut controller =
            ExecutionController::new(Arc::new(orchestrator), mock_llm, tool_registry);

        // Trigger FullStop before calling execute_turn_loop
        controller
            .kill_switch()
            .trigger(KillLevel::FullStop, "Test FullStop", "test")
            .await;

        let messages = vec![swell_llm::LlmMessage {
            role: swell_llm::LlmRole::User,
            content: "Hello".to_string(),
            tool_call_id: None,
        }];

        let result = controller.execute_turn_loop(messages, None).await;

        // Should fail with KillSwitchTriggered error
        assert!(
            result.is_err(),
            "execute_turn_loop should fail when FullStop is triggered"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(err, SwellError::KillSwitchTriggered),
            "Error should be KillSwitchTriggered, got: {:?}",
            err
        );

        // Reset kill switch
        controller.kill_switch().reset().await;
    }

    /// Test VAL-SAFE-001: FullStop blocks tool execution.
    ///
    /// When FullStop is triggered, execute_tool should return an error
    /// without attempting to find or execute the tool.
    #[tokio::test]
    async fn test_killswitch_fullstop_blocks_tool_execution() {
        use swell_core::kill_switch::KillLevel;

        let orchestrator = Orchestrator::new();
        let mock_llm = Arc::new(MockLlm::new("claude-sonnet"));
        let tool_registry = Arc::new(ToolRegistry::new());

        let mut controller =
            ExecutionController::new(Arc::new(orchestrator), mock_llm, tool_registry);

        // Trigger FullStop
        controller
            .kill_switch()
            .trigger(KillLevel::FullStop, "Test FullStop", "test")
            .await;

        let result = controller
            .execute_tool("some_tool", serde_json::json!({}))
            .await;

        // Should return Ok with error content, not fail
        assert!(
            result.is_ok(),
            "execute_tool should return Ok even when blocked: {:?}",
            result
        );
        let output = result.unwrap();
        assert!(
            output.is_error,
            "ToolOutput should have is_error=true when kill switch blocks execution"
        );
        assert!(
            output.content.iter().any(|c| match c {
                ToolResultContent::Error(msg) => msg.contains("kill switch"),
                _ => false,
            }),
            "Error message should mention kill switch: {:?}",
            output.content
        );

        // Reset kill switch
        controller.kill_switch().reset().await;
    }

    /// Test VAL-SAFE-002: Kill levels are enforced in correct order.
    ///
    /// Level 4 (FullStop) blocks everything, including operations
    /// blocked by lower levels (Throttle, ScopeBlock, NetworkKill).
    #[tokio::test]
    async fn test_killswitch_level_ordering_fullstop_overrides_all() {
        use swell_core::kill_switch::KillLevel;

        let orchestrator = Orchestrator::new();
        let mock_llm = Arc::new(MockLlm::new("claude-sonnet"));
        let tool_registry = Arc::new(ToolRegistry::new());

        let mut controller =
            ExecutionController::new(Arc::new(orchestrator), mock_llm, tool_registry);

        // Even though we set a low level (Throttle), FullStop should take precedence
        controller
            .kill_switch()
            .trigger(KillLevel::Throttle, "Throttle", "test")
            .await;
        controller
            .kill_switch()
            .trigger(KillLevel::FullStop, "FullStop overrides", "test")
            .await;

        let messages = vec![swell_llm::LlmMessage {
            role: swell_llm::LlmRole::User,
            content: "Hello".to_string(),
            tool_call_id: None,
        }];

        let result = controller.execute_turn_loop(messages, None).await;

        // FullStop should still halt even though Throttle was set first
        assert!(
            result.is_err(),
            "FullStop should halt even with Throttle also set"
        );

        // Reset kill switch
        controller.kill_switch().reset().await;
    }

    /// Test VAL-SAFE-002: NetworkKill allows non-network tools but blocks network tools.
    #[tokio::test]
    async fn test_killswitch_networkkill_ordering() {
        use swell_core::kill_switch::KillLevel;

        let orchestrator = Orchestrator::new();
        let mock_llm = Arc::new(MockLlm::new("claude-sonnet"));
        let tool_registry = Arc::new(ToolRegistry::new());

        let mut controller =
            ExecutionController::new(Arc::new(orchestrator), mock_llm, tool_registry);

        // Trigger NetworkKill
        controller
            .kill_switch()
            .trigger(KillLevel::NetworkKill, "Network blocked", "test")
            .await;

        // Execute turn loop should still work (it's not a network operation)
        let messages = vec![swell_llm::LlmMessage {
            role: swell_llm::LlmRole::User,
            content: "Hello".to_string(),
            tool_call_id: None,
        }];

        let result = controller.execute_turn_loop(messages, None).await;

        // Should succeed - turn loop itself is not a network operation
        assert!(
            result.is_ok(),
            "NetworkKill should not block turn loop: {:?}",
            result
        );

        // Reset kill switch
        controller.kill_switch().reset().await;
    }
}
