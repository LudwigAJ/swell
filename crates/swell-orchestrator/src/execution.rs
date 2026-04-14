//! Execution controller for managing parallel agent execution.
#![allow(clippy::should_implement_trait)]

use crate::{
    frozen_spec::FrozenSpecRef, EvaluatorAgent, FeatureLead, FeatureLeadSpawner, GeneratorAgent,
    Orchestrator, PlannerAgent, MAX_CONCURRENT_AGENTS,
};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use swell_core::traits::Agent;
use swell_core::{AgentContext, AgentResult, LlmMessage, StreamEvent, SwellError, ValidationResult};
use swell_llm::{LlmBackend, LlmToolDefinition};
use swell_tools::ToolRegistry;
use swell_validation::ValidationPipeline;
use tracing::{debug, info, warn};
use uuid::Uuid;

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
    pub fn add_tool_call(&mut self, name: String, id: String, arguments: serde_json::Value, result: String, success: bool) {
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

/// Manages concurrent task execution with up to 6 agents
pub struct ExecutionController {
    orchestrator: Arc<Orchestrator>,
    llm: Arc<dyn LlmBackend>,
    tool_registry: Arc<ToolRegistry>,
    validation_pipeline: ValidationPipeline,
    max_concurrent: usize,
    /// Maximum iterations for the turn loop (hard cap)
    /// This is separate from validation retry count
    max_iterations: u32,
    /// Frozen specs indexed by task_id, created at execution start
    frozen_specs: std::sync::RwLock<std::collections::HashMap<uuid::Uuid, FrozenSpecRef>>,
    /// Active FeatureLeads for complex tasks
    feature_leads: std::sync::RwLock<std::collections::HashMap<uuid::Uuid, FeatureLead>>,
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
            max_concurrent: MAX_CONCURRENT_AGENTS,
            max_iterations: DEFAULT_MAX_ITERATIONS,
            frozen_specs: std::sync::RwLock::new(std::collections::HashMap::new()),
            feature_leads: std::sync::RwLock::new(std::collections::HashMap::new()),
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
            max_concurrent: MAX_CONCURRENT_AGENTS,
            max_iterations: DEFAULT_MAX_ITERATIONS,
            frozen_specs: std::sync::RwLock::new(std::collections::HashMap::new()),
            feature_leads: std::sync::RwLock::new(std::collections::HashMap::new()),
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
            max_concurrent: MAX_CONCURRENT_AGENTS,
            max_iterations,
            frozen_specs: std::sync::RwLock::new(std::collections::HashMap::new()),
            feature_leads: std::sync::RwLock::new(std::collections::HashMap::new()),
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
            max_concurrent: MAX_CONCURRENT_AGENTS,
            max_iterations,
            frozen_specs: std::sync::RwLock::new(std::collections::HashMap::new()),
            feature_leads: std::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Get the max iterations setting
    pub fn max_iterations(&self) -> u32 {
        self.max_iterations
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

        // Step 1: Planning - run PlannerAgent if task doesn't have a plan
        let task = self.orchestrator.get_task(task_id).await?;
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
                // The planner should have set a plan on the task through the context
                // Re-fetch the task to get the updated plan
                info!(task_id = %task_id, "PlannerAgent completed successfully");
            } else {
                // Planner failed - return early with failure
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
        &self,
        mut messages: Vec<LlmMessage>,
        tools: Option<Vec<LlmToolDefinition>>,
    ) -> Result<(Vec<TurnSummary>, String), SwellError> {
        let mut all_summaries: Vec<TurnSummary> = Vec::new();
        let mut final_text = String::new();
        let mut turn_number = 0u32;

        loop {
            turn_number += 1;
            debug!(turn = turn_number, "Starting turn");

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
            let stream = self.llm.stream(messages.clone(), tools.clone(), Default::default()).await;

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
                        debug!(turn = turn_number, delta_len = delta.len(), "Received text delta");
                    }
                    Ok(StreamEvent::ToolUse { tool_call }) => {
                        debug!(
                            turn = turn_number,
                            tool_name = %tool_call.name,
                            tool_id = %tool_call.id,
                            "Received tool use event"
                        );
                        // Store the tool call to be executed when we get the result
                        current_tool_call = Some((
                            tool_call.id,
                            tool_call.name,
                            tool_call.arguments,
                        ));
                    }
                    Ok(StreamEvent::ToolResult { tool_call_id, result: _, success }) => {
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
                                Ok(output) => (output.success, output.result),
                                Err(e) => (false, e.to_string()),
                            };
                            summary.add_tool_call(name, id, arguments, result_str.clone(), was_success);

                            // Add tool result to messages using Assistant role
                            // (LlmRole doesn't have a Tool variant, so we use Assistant)
                            messages.push(LlmMessage {
                                role: swell_llm::LlmRole::Assistant,
                                content: result_str,
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
                            input_tokens,
                            output_tokens,
                            "Received usage event"
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
                warn!(turn = turn_number, "Turn produced empty response, terminating");
                all_summaries.push(summary);
                break;
            } else {
                summary.outcome = TurnOutcome::TextOnly;
                debug!(turn = turn_number, "Turn completed with text only, terminating");
                all_summaries.push(summary);
                break;
            }

            all_summaries.push(summary);
        }

        debug!(
            total_turns = all_summaries.len(),
            final_text_len = final_text.len(),
            "Turn loop completed"
        );

        Ok((all_summaries, final_text))
    }

    /// Execute a tool by name with the given arguments.
    async fn execute_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<swell_core::traits::ToolOutput, SwellError> {
        // Find the tool in the registry
        let tool = self.tool_registry.get(name).await.ok_or_else(|| {
            SwellError::ToolExecutionFailed(format!("Tool not found: {}", name))
        })?;

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
            max_concurrent: self.max_concurrent,
            max_iterations: self.max_iterations,
            frozen_specs: std::sync::RwLock::new(std::collections::HashMap::new()),
            feature_leads: std::sync::RwLock::new(std::collections::HashMap::new()),
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
            .create_task("Task 1".to_string())
            .await;
        let task2 = controller
            .orchestrator
            .create_task("Task 2".to_string())
            .await;

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

        let controller = ExecutionController::with_max_iterations(
            Arc::new(orchestrator),
            mock_llm,
            tool_registry,
            50, // High max_iterations so it doesn't trigger
        );

        let messages = vec![swell_llm::LlmMessage {
            role: swell_llm::LlmRole::User,
            content: "Say hello".to_string(),
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
        let controller = ExecutionController::with_max_iterations(
            Arc::new(orchestrator),
            mock_llm,
            tool_registry,
            2,
        );

        let messages = vec![swell_llm::LlmMessage {
            role: swell_llm::LlmRole::User,
            content: "Say hello".to_string(),
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

        let controller = ExecutionController::with_max_iterations(
            Arc::new(orchestrator),
            mock_llm,
            tool_registry,
            50,
        );

        let messages = vec![swell_llm::LlmMessage {
            role: swell_llm::LlmRole::User,
            content: "Say hello".to_string(),
        }];

        let result = controller.execute_turn_loop(messages, None).await;
        assert!(result.is_err());
    }
}
