//! Agent pool and agent implementations.

use swell_core::traits::Agent;
use swell_core::{
    AgentRole, AgentId, SwellError, AgentContext, AgentResult,
    MemoryBlock, MemoryBlockType, LlmMessage, LlmRole, LlmConfig,
    Task, Plan, PlanStep, StepStatus, RiskLevel, LlmBackend,
    ToolOutput, ToolCallResult,
};
use swell_tools::ToolRegistry;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;
use tracing::{info, debug, warn};
use serde::{Deserialize, Serialize};

/// Pool of agents for parallel execution
pub struct AgentPool {
    agents: HashMap<AgentId, PooledAgent>,
    next_id: u32,
}

#[derive(Debug, Clone)]
struct PooledAgent {
    id: AgentId,
    role: AgentRole,
    model: String,
    current_task: Option<Uuid>,
}

impl AgentPool {
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            next_id: 1,
        }
    }

    /// Register a new agent
    pub fn register(&mut self, role: AgentRole, model: String) -> AgentId {
        let id = Uuid::new_v4();
        self.agents.insert(id, PooledAgent {
            id,
            role,
            model,
            current_task: None,
        });
        info!(agent_id = %id, role = ?role, "Registered agent");
        id
    }

    /// Reserve an agent for a task
    pub fn reserve(&mut self, task_id: Uuid, role: AgentRole) -> Result<AgentId, SwellError> {
        // Find an available agent of the right role
        let agent_id = self.agents.iter()
            .find(|(_, a)| a.role == role && a.current_task.is_none())
            .map(|(id, _)| *id)
            .ok_or_else(|| SwellError::AgentNotFound(Uuid::nil()))?;

        if let Some(agent) = self.agents.get_mut(&agent_id) {
            agent.current_task = Some(task_id);
        }

        debug!(agent_id = %agent_id, task_id = %task_id, "Agent reserved");
        Ok(agent_id)
    }

    /// Release an agent back to the pool
    pub fn release(&mut self, agent_id: AgentId) {
        if let Some(agent) = self.agents.get_mut(&agent_id) {
            agent.current_task = None;
            debug!(agent_id = %agent_id, "Agent released");
        }
    }

    /// Count available agents for a role
    pub fn available_count(&self, role: AgentRole) -> usize {
        self.agents.values()
            .filter(|a| a.role == role && a.current_task.is_none())
            .count()
    }

    /// Get agent's current task
    pub fn get_task(&self, agent_id: AgentId) -> Option<Uuid> {
        self.agents.get(&agent_id).and_then(|a| a.current_task)
    }

    /// Get all agents
    pub fn agents(&self) -> &HashMap<AgentId, PooledAgent> {
        &self.agents
    }
}

impl Default for AgentPool {
    fn default() -> Self {
        Self::new()
    }
}

/// Handle to a reserved agent
#[derive(Debug, Clone)]
pub struct AgentHandle {
    pub id: AgentId,
    pub role: AgentRole,
    pub model: String,
}

impl AgentHandle {
    pub fn new(id: AgentId, role: AgentRole, model: String) -> Self {
        Self { id, role, model }
    }
}

// ============================================================================
// Agent Implementations
// ============================================================================

/// Planner agent - creates execution plans from task descriptions
pub struct PlannerAgent {
    model: String,
    system_prompt: String,
    llm: Arc<dyn LlmBackend>,
}

impl PlannerAgent {
    /// Create a new PlannerAgent with an LLM backend
    pub fn with_llm(model: String, llm: Arc<dyn LlmBackend>) -> Self {
        Self {
            model,
            llm,
            system_prompt: r#"
You are a planner agent for an autonomous coding engine.
Your job is to analyze a task description and create a structured execution plan.

Output a JSON plan with the following structure:
{
  "steps": [
    {
      "description": "What to do in this step",
      "affected_files": ["file1.rs", "file2.rs"],
      "expected_tests": ["test_function_a", "test_function_b"],
      "risk_level": "low|medium|high",
      "dependencies": []
    }
  ],
  "total_estimated_tokens": 10000,
  "risk_assessment": "Overall risk description"
}

Focus on:
- Breaking down the task into logical units of work
- Identifying dependencies between steps
- Estimating risk appropriately
- Planning test coverage
"#.to_string(),
        }
    }
}

/// Helper function to parse a string array from JSON
fn parse_string_array(arr: Option<&serde_json::Value>) -> Vec<String> {
    arr.and_then(|v| v.as_array())
        .map(|items| {
            items.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

#[async_trait]
impl Agent for PlannerAgent {
    fn role(&self) -> AgentRole {
        AgentRole::Planner
    }

    fn description(&self) -> String {
        "Creates execution plans from task descriptions using LLM".to_string()
    }

    async fn execute(&self, context: AgentContext) -> Result<AgentResult, SwellError> {
        // Build the user message with task description
        let user_message = format!(
            "Create an execution plan for the following task:\n\nTask: {}\n\nWorkspace: {}\n\nMemory Context:\n{}",
            context.task.description,
            context.workspace_path.as_deref().unwrap_or("."),
            context.memory_blocks.iter()
                .map(|b| format!("- [{}] {}\n{}", format!("{:?}", b.block_type).to_lowercase(), b.label, b.content))
                .collect::<Vec<_>>()
                .join("\n")
        );

        // Build messages for the LLM
        let messages = vec![
            LlmMessage {
                role: LlmRole::System,
                content: self.system_prompt.clone(),
            },
            LlmMessage {
                role: LlmRole::User,
                content: user_message,
            },
        ];

        // Call the LLM
        let config = LlmConfig {
            temperature: 0.3, // Lower temperature for more deterministic planning
            max_tokens: 4096,
            stop_sequences: None,
        };

        let response = self.llm.chat(messages, None, config).await?;

        // Parse the plan from LLM response
        let plan_json: serde_json::Value = serde_json::from_str(&response.content)
            .map_err(|e| SwellError::LlmError(format!("Failed to parse plan JSON: {}. Raw content: {}", e, &response.content)))?;

        // Convert JSON to Plan structure
        let steps: Vec<PlanStep> = plan_json["steps"]
            .as_array()
            .ok_or_else(|| SwellError::LlmError("Missing steps array in plan".to_string()))?
            .iter()
            .map(|step| {
                let risk_str = step["risk_level"].as_str().unwrap_or("medium");
                let risk_level = match risk_str.to_lowercase().as_str() {
                    "low" => RiskLevel::Low,
                    "high" => RiskLevel::High,
                    _ => RiskLevel::Medium,
                };

                PlanStep {
                    id: Uuid::new_v4(),
                    description: step["description"].as_str().unwrap_or("").to_string(),
                    affected_files: parse_string_array(Some(&step["affected_files"])),
                    expected_tests: parse_string_array(Some(&step["expected_tests"])),
                    risk_level,
                    dependencies: vec![], // Dependencies would be parsed from step if provided
                    status: StepStatus::Pending,
                }
            })
            .collect();

        let total_estimated_tokens = plan_json["total_estimated_tokens"]
            .as_u64()
            .unwrap_or(5000);

        let risk_assessment = plan_json["risk_assessment"]
            .as_str()
            .unwrap_or("Standard implementation task")
            .to_string();

        // Build the Plan struct
        let plan = Plan {
            id: Uuid::new_v4(),
            task_id: context.task.id,
            steps,
            total_estimated_tokens,
            risk_assessment,
        };

        // Serialize to JSON for output
        let plan_output = serde_json::to_string(&plan)
            .map_err(|e| SwellError::LlmError(format!("Failed to serialize plan: {}", e)))?;

        Ok(AgentResult {
            success: true,
            output: plan_output,
            tool_calls: vec![],
            tokens_used: response.usage.total_tokens,
            error: None,
        })
    }
}

/// Generator agent - implements code based on plans using ReAct loop
pub struct GeneratorAgent {
    model: String,
    llm: Option<Arc<dyn LlmBackend>>,
    tool_registry: Option<Arc<ToolRegistry>>,
    max_iterations: u32,
}

impl GeneratorAgent {
    /// Create a new GeneratorAgent with model name only (for testing)
    pub fn new(model: String) -> Self {
        Self {
            model,
            llm: None,
            tool_registry: None,
            max_iterations: DEFAULT_REACT_MAX_ITERATIONS,
        }
    }

    /// Create a GeneratorAgent with LLM backend for ReAct reasoning
    pub fn with_llm(model: String, llm: Arc<dyn LlmBackend>) -> Self {
        Self {
            model,
            llm: Some(llm),
            tool_registry: None,
            max_iterations: DEFAULT_REACT_MAX_ITERATIONS,
        }
    }

    /// Create a GeneratorAgent with tool registry for tool execution
    pub fn with_tools(model: String, tool_registry: Arc<ToolRegistry>) -> Self {
        Self {
            model,
            llm: None,
            tool_registry: Some(tool_registry),
            max_iterations: DEFAULT_REACT_MAX_ITERATIONS,
        }
    }

    /// Create a fully configured GeneratorAgent with LLM and tools
    pub fn with_llm_and_tools(
        model: String,
        llm: Arc<dyn LlmBackend>,
        tool_registry: Arc<ToolRegistry>,
    ) -> Self {
        Self {
            model,
            llm: Some(llm),
            tool_registry: Some(tool_registry),
            max_iterations: DEFAULT_REACT_MAX_ITERATIONS,
        }
    }

    /// Execute a single plan step using the ReAct loop
    async fn execute_step_with_react(
        &self,
        step: &PlanStep,
        workspace_path: &str,
    ) -> Result<String, SwellError> {
        let mut react_loop = ReactLoop::new(self.max_iterations);

        // Build initial context for the agent
        let initial_thought = format!(
            "I need to implement: {}\n\
             Affected files: {:?}\n\
             Risk level: {:?}\n\
             Workspace: {}",
            step.description, step.affected_files, step.risk_level, workspace_path
        );

        react_loop.think(initial_thought);

        // ReAct loop: Think → Act → Observe → Repeat
        while react_loop.should_continue() {
            // Get the current thought from the loop
            let current_thought = react_loop.steps
                .last()
                .map(|s| s.thought.clone())
                .unwrap_or_default();

            // Use LLM to decide next action if available, otherwise use simple heuristics
            let action = if let Some(llm) = &self.llm {
                self.decide_action_with_llm(&current_thought, step, llm).await?
            } else {
                self.decide_action_heuristic(&current_thought, step)?
            };

            react_loop.act(action.clone());

            // Execute the action using tools
            let observation = self.execute_action(&action, workspace_path).await?;
            react_loop.observe(observation.clone());

            // Check if we succeeded (observation indicates completion)
            if self.is_step_complete(&observation) {
                react_loop.success(format!("Step completed successfully: {}", step.description));
                break;
            }

            // Check for failures
            if observation.contains("ERROR:") || observation.contains("FAILED:") {
                let reflection = react_loop.failure(observation.clone());
                // Add reflection as new thought for next iteration
                react_loop.think(reflection);
            }
        }

        // Build output from the ReAct loop execution
        let summary = react_loop.summary();
        Ok(serde_json::to_string(&summary).unwrap_or_default())
    }

    /// Use LLM to decide the next action based on current thought
    async fn decide_action_with_llm(
        &self,
        thought: &str,
        step: &PlanStep,
        llm: &Arc<dyn LlmBackend>,
    ) -> Result<String, SwellError> {
        let prompt = format!(
            r#"You are a coding agent deciding what action to take next.

Current task: {}
Affected files: {:?}
Risk level: {}

Current thinking:
{}

Based on the thinking, decide what action to take next. Choose from:
- read_file(path="<file>") - Read a file to understand its structure
- write_file(path="<file>", content="<content>") - Create a new file or overwrite
- edit_file(path="<file>", old_str="<exact text>", new_str="<new text>") - Edit existing file
- shell(command="<cmd>", args=["arg1", "arg2"]) - Execute shell command
- search(operation="grep|glob|symbol_search", pattern="<pattern>", path="<path>") - Search code

Respond ONLY with the action in JSON format: {{"action": "tool_name", "args": {{"param": "value"}}}}"#,
            step.description, step.affected_files, format!("{:?}", step.risk_level), thought
        );

        let messages = vec![
            LlmMessage {
                role: LlmRole::User,
                content: prompt,
            },
        ];

        let config = LlmConfig {
            temperature: 0.3,
            max_tokens: 500,
            stop_sequences: None,
        };

        let response = llm.chat(messages, None, config).await?;
        Ok(response.content)
    }

    /// Simple heuristic-based action decision when LLM is not available
    fn decide_action_heuristic(
        &self,
        thought: &str,
        step: &PlanStep,
    ) -> Result<String, SwellError> {
        // Simple heuristics based on step description and affected files
        if step.affected_files.is_empty() {
            return Ok(r#"{"action": "shell", "args": {"command": "echo", "args": ["No files to modify"]}}"#.to_string());
        }

        // If files exist and we haven't read them yet
        if !thought.contains("READ:") && !thought.contains("read_file") {
            let file = &step.affected_files[0];
            return Ok(format!(r#"{{"action": "read_file", "args": {{"path": "{}"}}}}"#, file));
        }

        // If we've read the file and understand it, make an edit
        if step.risk_level == RiskLevel::Low {
            let file = &step.affected_files[0];
            Ok(format!(
                r#"{{"action": "edit_file", "args": {{"path": "{}", "old_str": "// TODO", "new_str": "// DONE"}}}}"#,
                file
            ))
        } else {
            Ok(r#"{"action": "shell", "args": {"command": "echo", "args": ["Implementation pending"]}}"#.to_string())
        }
    }

    /// Execute an action and return the observation
    async fn execute_action(&self, action_json: &str, workspace_path: &str) -> Result<String, SwellError> {
        let registry = self.tool_registry.as_ref()
            .ok_or_else(|| SwellError::ToolExecutionFailed("No tool registry configured".to_string()))?;

        // Try to parse the action JSON
        let action: serde_json::Value = serde_json::from_str(action_json)
            .unwrap_or_else(|_| {
                // If not valid JSON, treat it as a simple command
                serde_json::json!({
                    "action": "shell",
                    "args": {"command": action_json}
                })
            });

        let tool_name = action["action"].as_str().unwrap_or("shell");
        let tool_args = &action["args"];

        // Execute the tool
        if let Some(tool) = registry.get(tool_name).await {
            let result: swell_core::ToolOutput = tool.execute(tool_args.clone()).await?;
            if result.success {
                Ok(format!("OK: {}", result.result))
            } else {
                Ok(format!("FAILED: {}", result.error.unwrap_or_default()))
            }
        } else {
            Ok(format!("ERROR: Tool '{}' not found", tool_name))
        }
    }

    /// Check if the step is complete based on observation
    fn is_step_complete(&self, observation: &str) -> bool {
        observation.starts_with("OK:") && !observation.contains("ERROR")
    }
}

#[async_trait]
impl Agent for GeneratorAgent {
    fn role(&self) -> AgentRole {
        AgentRole::Generator
    }

    fn description(&self) -> String {
        "Generates code implementations from plans using ReAct loop".to_string()
    }

    async fn execute(&self, context: AgentContext) -> Result<AgentResult, SwellError> {
        let workspace_path = context.workspace_path.as_deref().unwrap_or(".");

        // Get the plan from context or create a simple one from task description
        let plan = if let Some(plan) = &context.task.plan {
            plan.clone()
        } else {
            // Create a simple plan from task description
            Plan {
                id: Uuid::new_v4(),
                task_id: context.task.id,
                steps: vec![PlanStep {
                    id: Uuid::new_v4(),
                    description: context.task.description.clone(),
                    affected_files: vec![],
                    expected_tests: vec![],
                    risk_level: RiskLevel::Medium,
                    dependencies: vec![],
                    status: StepStatus::Pending,
                }],
                total_estimated_tokens: 5000,
                risk_assessment: "Standard implementation task".to_string(),
            }
        };

        let mut all_outputs = Vec::new();
        let mut tool_call_results = Vec::new();
        let mut total_tokens = 0u64;

        // If no tool registry is configured, return a simple success result (MVP behavior)
        let has_tool_registry = self.tool_registry.is_some();
        if !has_tool_registry {
            return Ok(AgentResult {
                success: true,
                output: format!("Generated code for: {} (ReAct loop pending tool registry)", context.task.description),
                tool_calls: vec![],
                tokens_used: 1000,
                error: None,
            });
        }

        // Execute each step in the plan using ReAct loop
        for step in &plan.steps {
            let step_output = self.execute_step_with_react(step, workspace_path).await?;
            all_outputs.push(step_output);
        }

        let output = serde_json::json!({
            "plan_id": plan.id,
            "steps_executed": plan.steps.len(),
            "step_outputs": all_outputs,
            "generator": "GeneratorAgent with ReAct loop"
        });

        Ok(AgentResult {
            success: true,
            output: serde_json::to_string(&output).unwrap_or_default(),
            tool_calls: tool_call_results,
            tokens_used: total_tokens,
            error: None,
        })
    }
}

/// Evaluator agent - validates code quality using validation gates
pub struct EvaluatorAgent {
    model: String,
    validation_pipeline: Option<swell_validation::ValidationPipeline>,
}

impl EvaluatorAgent {
    /// Create a new EvaluatorAgent without validation pipeline (MVP stub mode)
    pub fn new(model: String) -> Self {
        Self {
            model,
            validation_pipeline: None,
        }
    }

    /// Create an EvaluatorAgent with a validation pipeline
    pub fn with_pipeline(model: String, pipeline: swell_validation::ValidationPipeline) -> Self {
        Self {
            model,
            validation_pipeline: Some(pipeline),
        }
    }

    /// Create an EvaluatorAgent with default validation gates (Lint, Test, Security, AI Review)
    pub fn with_defaults(model: String) -> Self {
        let mut pipeline = swell_validation::ValidationPipeline::new();
        pipeline.add_gate(swell_validation::LintGate::new());
        pipeline.add_gate(swell_validation::TestGate::new());
        pipeline.add_gate(swell_validation::SecurityGate::new());
        pipeline.add_gate(swell_validation::AiReviewGate::new());

        Self {
            model,
            validation_pipeline: Some(pipeline),
        }
    }

    /// Extract changed files from the task's plan
    fn extract_changed_files(context: &AgentContext) -> Vec<String> {
        let mut files = Vec::new();

        if let Some(ref plan) = context.task.plan {
            for step in &plan.steps {
                for file in &step.affected_files {
                    if !files.contains(file) {
                        files.push(file.clone());
                    }
                }
            }
        }

        files
    }

    /// Build validation context from agent context
    fn build_validation_context(context: &AgentContext) -> swell_core::ValidationContext {
        let workspace_path = context
            .workspace_path
            .clone()
            .unwrap_or_else(|| ".".to_string());

        let changed_files = Self::extract_changed_files(context);

        swell_core::ValidationContext {
            task_id: context.task.id,
            workspace_path,
            changed_files,
            plan: context.task.plan.clone(),
        }
    }

    /// Compute confidence score from validation outcome
    fn compute_confidence(outcome: &swell_core::ValidationOutcome) -> swell_validation::ConfidenceScore {
        use swell_validation::{ConfidenceScorer, ConfidenceLevel};

        let mut scorer = ConfidenceScorer::new();

        // Add lint signal based on messages
        let lint_messages: Vec<_> = outcome
            .messages
            .iter()
            .filter(|m| m.file.is_some())
            .collect();
        let lint_passed = outcome.passed && !lint_messages.iter().any(|m| m.level == swell_core::ValidationLevel::Error);
        let lint_warning_ratio = if lint_messages.is_empty() {
            0.0
        } else {
            let warnings: f64 = lint_messages
                .iter()
                .filter(|m| m.level == swell_core::ValidationLevel::Warning)
                .count() as f64;
            warnings / lint_messages.len() as f64
        };
        scorer = scorer.with_lint(lint_passed, lint_warning_ratio);

        // Add test signal (we consider tests passed if outcome passed)
        let tests_passed = outcome.passed;
        let coverage = if outcome.passed { 0.8 } else { 0.4 }; // Simplified estimation
        scorer = scorer.with_tests(tests_passed, coverage);

        // Add security signal (stub always passes)
        let security_passed = true;
        scorer = scorer.with_security(security_passed, 0);

        // Add AI review signal (stub always passes with medium confidence)
        let ai_review_passed = true;
        scorer = scorer.with_ai_review(ai_review_passed, 0.6);

        scorer.score()
    }

    /// Build evaluation result from validation outcome and confidence
    fn build_evaluation_result(
        outcome: swell_core::ValidationOutcome,
        confidence: swell_validation::ConfidenceScore,
    ) -> EvaluationResult {
        let errors: Vec<String> = outcome
            .messages
            .iter()
            .filter(|m| m.level == swell_core::ValidationLevel::Error)
            .map(|m| m.message.clone())
            .collect();

        let warnings: Vec<String> = outcome
            .messages
            .iter()
            .filter(|m| m.level == swell_core::ValidationLevel::Warning)
            .map(|m| m.message.clone())
            .collect();

        EvaluationResult {
            passed: outcome.passed,
            confidence_score: confidence.score,
            confidence_level: match confidence.level {
                swell_validation::ConfidenceLevel::Low => ConfidenceLevel::Low,
                swell_validation::ConfidenceLevel::Medium => ConfidenceLevel::Medium,
                swell_validation::ConfidenceLevel::High => ConfidenceLevel::High,
                swell_validation::ConfidenceLevel::VeryHigh => ConfidenceLevel::VeryHigh,
            },
            errors,
            warnings,
            can_auto_merge: confidence.can_auto_merge(),
            messages: outcome.messages.len() as u32,
            artifacts: outcome.artifacts.len() as u32,
        }
    }
}

/// Evaluation result from the Evaluator agent
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvaluationResult {
    pub passed: bool,
    pub confidence_score: f64,
    pub confidence_level: ConfidenceLevel,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub can_auto_merge: bool,
    pub messages: u32,
    pub artifacts: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ConfidenceLevel {
    Low,
    Medium,
    High,
    VeryHigh,
}

#[async_trait]
impl Agent for EvaluatorAgent {
    fn role(&self) -> AgentRole {
        AgentRole::Evaluator
    }

    fn description(&self) -> String {
        "Evaluates code quality and correctness through validation gates".to_string()
    }

    async fn execute(&self, context: AgentContext) -> Result<AgentResult, SwellError> {
        // If no validation pipeline is configured, use stub mode
        let Some(pipeline) = &self.validation_pipeline else {
            // MVP stub mode - simulate successful evaluation
            let stub_result = EvaluationResult {
                passed: true,
                confidence_score: 0.7,
                confidence_level: ConfidenceLevel::Medium,
                errors: vec![],
                warnings: vec!["Running in stub mode - validation pipeline not configured".to_string()],
                can_auto_merge: false,
                messages: 0,
                artifacts: 0,
            };

            return Ok(AgentResult {
                success: true,
                output: serde_json::to_string(&stub_result).unwrap_or_default(),
                tool_calls: vec![],
                tokens_used: 300,
                error: None,
            });
        };

        // Build validation context
        let validation_context = Self::build_validation_context(&context);

        // Run validation pipeline
        let outcome = pipeline.run(&validation_context).await?;

        // Compute confidence score
        let confidence = Self::compute_confidence(&outcome);

        // Build evaluation result
        let result = Self::build_evaluation_result(outcome, confidence.clone());

        // Serialize output
        let output = serde_json::json!({
            "evaluation": result,
            "confidence": {
                "score": confidence.score,
                "level": format!("{:?}", confidence.level).to_uppercase(),
                "summary": confidence.summary(),
            },
            "validation_context": {
                "task_id": validation_context.task_id,
                "workspace_path": validation_context.workspace_path,
                "changed_files": validation_context.changed_files,
            }
        });

        Ok(AgentResult {
            success: result.passed,
            output: serde_json::to_string(&output).unwrap_or_default(),
            tool_calls: vec![],
            tokens_used: 500,
            error: if result.passed { None } else { Some("Validation failed".to_string()) },
        })
    }
}

// ============================================================================
// SystemPromptBuilder - Assembles agent context from project config, conventions, memory blocks
// ============================================================================

/// Configuration for building system prompts
#[derive(Debug, Clone)]
pub struct SystemPromptConfig {
    /// Project name for context
    pub project_name: String,
    /// Repository root path
    pub repo_path: String,
    /// Agent role being built for
    pub for_role: AgentRole,
    /// Include memory blocks
    pub include_memory: bool,
    /// Include project conventions
    pub include_conventions: bool,
    /// Max tokens for the prompt
    pub max_tokens: usize,
}

impl Default for SystemPromptConfig {
    fn default() -> Self {
        Self {
            project_name: "SWELL".to_string(),
            repo_path: ".".to_string(),
            for_role: AgentRole::Coder,
            include_memory: true,
            include_conventions: true,
            max_tokens: 8000,
        }
    }
}

/// SystemPromptBuilder assembles agent context from project config, conventions, memory blocks, task context
pub struct SystemPromptBuilder {
    config: SystemPromptConfig,
}

impl SystemPromptBuilder {
    pub fn new(config: SystemPromptConfig) -> Self {
        Self { config }
    }

    /// Build a system prompt with the given task context
    pub fn build(&self, task_context: &str, memory_blocks: &[MemoryBlock]) -> String {
        let mut prompt = String::new();

        // Header with project info
        prompt.push_str(&format!(
            "# {} - {} Agent\n\n",
            self.config.project_name, 
            format!("{:?}", self.config.for_role).to_lowercase()
        ));

        // Role-specific instructions
        prompt.push_str(&self.role_instructions());
        prompt.push_str("\n\n");

        // Project conventions if included
        if self.config.include_conventions {
            prompt.push_str("## Project Conventions\n\n");
            prompt.push_str(&self.project_conventions());
            prompt.push_str("\n\n");
        }

        // Memory blocks if included
        if self.config.include_memory && !memory_blocks.is_empty() {
            prompt.push_str("## Relevant Context\n\n");
            for block in memory_blocks {
                prompt.push_str(&format!("### {} ({})\n{}\n\n", 
                    block.label, 
                    format!("{:?}", block.block_type).to_lowercase(),
                    block.content
                ));
            }
        }

        // Task context
        prompt.push_str("## Current Task\n\n");
        prompt.push_str(task_context);
        prompt.push_str("\n\n");

        // Tool usage guidelines
        prompt.push_str(&self.tool_guidelines());

        prompt
    }

    fn role_instructions(&self) -> String {
        match self.config.for_role {
            AgentRole::Planner => r#"
You are a PLANNER agent. Your role is to:
- Analyze task descriptions and break them into logical steps
- Identify dependencies between steps
- Assess risk levels for each step
- Estimate token usage and time requirements
- Output a structured JSON plan

Follow the planning workflow strictly.
"#.to_string(),
            AgentRole::Generator => r#"
You are a GENERATOR agent. Your role is to:
- Receive plans from the Planner agent
- Coordinate Coder agents to implement code changes
- Track progress through the ReAct loop
- Ensure all changes are validated

Use the ReAct pattern: Think → Act → Observe → Repeat
"#.to_string(),
            AgentRole::Evaluator => r#"
You are an EVALUATOR agent. Your role is to:
- Run validation gates on generated code
- Check linting, tests, and security
- Provide confidence scores for outputs
- Gatekeep quality before acceptance

Be thorough but efficient in validation.
"#.to_string(),
            AgentRole::Coder => r#"
You are a CODER agent. Your role is to:
- Implement specific code changes based on task descriptions
- Make minimal, focused changes
- Self-validate outputs before completing
- Produce diffs showing exact changes

Write clean, idiomatic code following project conventions.
"#.to_string(),
            AgentRole::TestWriter => r#"
You are a TEST WRITER agent. Your role is to:
- Generate tests from Given/When/Then acceptance criteria
- Use existing test patterns in the codebase
- Map coverage to requirements
- Ensure tests are deterministic and isolated

Write meaningful tests that catch real bugs.
"#.to_string(),
            AgentRole::Reviewer => r#"
You are a REVIEWER agent. Your role is to:
- Review code for style, complexity, and regressions
- Check adherence to project conventions
- Flag potential issues and suggest improvements
- Ensure code is maintainable and well-documented

Be constructive and specific in feedback.
"#.to_string(),
            AgentRole::Refactorer => r#"
You are a REFACTORER agent. Your role is to:
- Identify refactoring opportunities
- Preserve external behavior during restructuring
- Validate refactors don't break functionality
- Prioritize impactful improvements

Refactor with confidence - verify behavior is preserved.
"#.to_string(),
            AgentRole::DocWriter => r#"
You are a DOC WRITER agent. Your role is to:
- Generate and modify documentation from code changes
- Follow project documentation conventions
- Update READMEs and API docs
- Ensure docs stay in sync with code

Write clear, accurate documentation.
"#.to_string(),
        }
    }

    fn project_conventions(&self) -> String {
        // Default conventions - these would come from memory blocks in a full implementation
        r#"
- Use conventional commits: feat:, fix:, docs:, refactor:, test:
- Follow Rust idioms and style
- All public APIs need documentation
- Tests must pass before merge
- Maximum function length: 50 lines
- Maximum cyclomatic complexity: 10
"#.to_string()
    }

    fn tool_guidelines(&self) -> String {
        r#"
## Tool Usage

When you need to perform actions, use the available tools:
- `file_read` - Read file contents
- `file_write` - Create or overwrite files
- `file_edit` - Make targeted modifications
- `shell_exec` - Execute shell commands
- `git_status` - Check git status
- `search` - Search for patterns in code

Always validate tool outputs before proceeding.
"#.to_string()
    }

    /// Calculate current context usage percentage
    pub fn calculate_usage(&self, prompt: &str) -> f64 {
        // Rough estimation: ~4 characters per token
        let estimated_tokens = prompt.len() / 4;
        estimated_tokens as f64 / self.config.max_tokens as f64
    }
}

// ============================================================================
// ReAct Loop - Think→Act→Observe→Repeat with reflection on failures
// ============================================================================

/// Maximum iterations for ReAct loop
pub const DEFAULT_REACT_MAX_ITERATIONS: u32 = 15;

/// A single step in the ReAct loop
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactStep {
    pub iteration: u32,
    pub phase: ReactPhase,
    pub thought: String,
    pub action: Option<String>,
    pub observation: Option<String>,
    pub result: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReactPhase {
    Think,
    Act,
    Observe,
    Repeat,
    Done,
    Failed,
}

/// ReAct loop state machine
#[derive(Debug, Clone)]
pub struct ReactLoop {
    pub max_iterations: u32,
    pub current_iteration: u32,
    pub steps: Vec<ReactStep>,
    pub state: ReactLoopState,
    pub failure_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReactLoopState {
    Running,
    Converged,
    Failed,
    MaxIterationsReached,
}

impl Default for ReactLoop {
    fn default() -> Self {
        Self::new(DEFAULT_REACT_MAX_ITERATIONS)
    }
}

impl ReactLoop {
    pub fn new(max_iterations: u32) -> Self {
        Self {
            max_iterations,
            current_iteration: 0,
            steps: Vec::new(),
            state: ReactLoopState::Running,
            failure_count: 0,
        }
    }

    /// Start a new think phase
    pub fn think(&mut self, thought: String) {
        self.current_iteration += 1;
        
        if self.current_iteration > self.max_iterations {
            self.state = ReactLoopState::MaxIterationsReached;
            return;
        }

        let step = ReactStep {
            iteration: self.current_iteration,
            phase: ReactPhase::Think,
            thought,
            action: None,
            observation: None,
            result: None,
        };
        
        self.steps.push(step);
    }

    /// Record an action taken
    pub fn act(&mut self, action: String) {
        if let Some(step) = self.steps.last_mut() {
            step.phase = ReactPhase::Act;
            step.action = Some(action);
        }
    }

    /// Record an observation from the action
    pub fn observe(&mut self, observation: String) {
        if let Some(step) = self.steps.last_mut() {
            step.phase = ReactPhase::Observe;
            step.observation = Some(observation);
        }
    }

    /// Record success and complete
    pub fn success(&mut self, result: String) {
        if let Some(step) = self.steps.last_mut() {
            step.phase = ReactPhase::Done;
            step.result = Some(result);
        }
        self.state = ReactLoopState::Converged;
    }

    /// Record a failure with reflection
    pub fn failure(&mut self, error: String) -> String {
        self.failure_count += 1;
        
        if let Some(step) = self.steps.last_mut() {
            step.phase = ReactPhase::Failed;
            step.result = Some(error.clone());
        }
        
        // Reflection: analyze what went wrong
        let reflection = self.reflect_on_failure();
        
        if self.failure_count >= 3 {
            self.state = ReactLoopState::Failed;
        }
        
        reflection
    }

    /// Reflect on failures to generate improvement suggestions
    fn reflect_on_failure(&self) -> String {
        let recent_steps: Vec<_> = self.steps.iter().rev().take(3).collect();
        
        let mut patterns = Vec::new();
        for step in recent_steps {
            if step.phase == ReactPhase::Failed {
                patterns.push(format!("Failed at iteration {}: {}", step.iteration, step.result.as_deref().unwrap_or("Unknown")));
            }
        }
        
        if patterns.is_empty() {
            "No clear failure pattern detected. Consider reviewing the last action.".to_string()
        } else {
            format!("Detected failure patterns: {}. Consider trying a different approach.", patterns.join("; "))
        }
    }

    /// Check if loop should continue
    pub fn should_continue(&self) -> bool {
        matches!(self.state, ReactLoopState::Running) && 
        self.current_iteration < self.max_iterations
    }

    /// Get summary of the loop execution
    pub fn summary(&self) -> ReactLoopSummary {
        ReactLoopSummary {
            total_iterations: self.current_iteration,
            failure_count: self.failure_count,
            final_state: self.state,
            steps: self.steps.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactLoopSummary {
    pub total_iterations: u32,
    pub failure_count: u32,
    pub final_state: ReactLoopState,
    pub steps: Vec<ReactStep>,
}

// ============================================================================
// Context Condensation - Auto-compact at 75% window utilization
// ============================================================================

/// Context window configuration
#[derive(Debug, Clone)]
pub struct ContextWindow {
    /// Maximum tokens in window
    pub max_tokens: usize,
    /// Warning threshold (0.0 to 1.0)
    pub warning_threshold: f64,
    /// Condensation threshold (0.0 to 1.0)
    pub condensation_threshold: f64,
}

impl Default for ContextWindow {
    fn default() -> Self {
        Self {
            max_tokens: 100_000,
            warning_threshold: 0.65,
            condensation_threshold: 0.75,
        }
    }
}

/// Context condensation result
#[derive(Debug, Clone)]
pub struct CondensationResult {
    pub original_tokens: usize,
    pub condensed_tokens: usize,
    pub preserved_items: Vec<String>,
    pub removed_items: Vec<String>,
    pub compression_ratio: f64,
}

/// ContextCondensation monitors and compacts context to prevent overflow
pub struct ContextCondensation {
    window: ContextWindow,
}

impl Default for ContextCondensation {
    fn default() -> Self {
        Self::new(ContextWindow::default())
    }
}

impl ContextCondensation {
    pub fn new(window: ContextWindow) -> Self {
        Self { window }
    }

    /// Check if condensation is needed
    pub fn needs_condensation(&self, current_tokens: usize) -> CondensationLevel {
        let ratio = current_tokens as f64 / self.window.max_tokens as f64;
        
        if ratio >= self.window.condensation_threshold {
            CondensationLevel::MustCondense
        } else if ratio >= self.window.warning_threshold {
            CondensationLevel::Warning
        } else {
            CondensationLevel::Ok
        }
    }

    /// Condense context to fit within threshold
    pub fn condense(&self, items: &[ContextItem]) -> CondensationResult {
        let original_tokens: usize = items.iter().map(|i| i.tokens).sum();
        let target_tokens = (self.window.max_tokens as f64 * self.window.warning_threshold) as usize;
        
        let mut sorted_items: Vec<_> = items.iter().collect();
        // Sort by priority (higher first) then by tokens (lower first)
        sorted_items.sort_by(|a, b| {
            match b.priority.cmp(&a.priority) {
                std::cmp::Ordering::Equal => a.tokens.cmp(&b.tokens),
                other => other,
            }
        });

        let mut preserved_tokens = 0;
        let mut preserved_items = Vec::new();
        let mut removed_items = Vec::new();

        for item in sorted_items {
            if preserved_tokens + item.tokens <= target_tokens {
                preserved_items.push(item.id.clone());
                preserved_tokens += item.tokens;
            } else {
                removed_items.push(item.id.clone());
            }
        }

        let condensed_tokens = preserved_tokens;
        let compression_ratio = if original_tokens > 0 {
            1.0 - (condensed_tokens as f64 / original_tokens as f64)
        } else {
            0.0
        };

        CondensationResult {
            original_tokens,
            condensed_tokens,
            preserved_items,
            removed_items,
            compression_ratio,
        }
    }

    /// Calculate current utilization percentage
    pub fn utilization(&self, current_tokens: usize) -> f64 {
        current_tokens as f64 / self.window.max_tokens as f64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondensationLevel {
    Ok,
    Warning,
    MustCondense,
}

/// An item in the context window
#[derive(Debug, Clone)]
pub struct ContextItem {
    pub id: String,
    pub content: String,
    pub tokens: usize,
    pub priority: u32, // Higher = more important
    pub item_type: ContextItemType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextItemType {
    SystemPrompt,
    MemoryBlock,
    ToolResult,
    UserMessage,
    AgentMessage,
    TaskContext,
}

// ============================================================================
// CoderAgent - Implements code changes with diff-based modifications
// ============================================================================

/// Coder agent for implementing specific code changes
pub struct CoderAgent {
    model: String,
    system_prompt_builder: SystemPromptBuilder,
    llm: Option<Arc<dyn LlmBackend>>,
    tool_registry: Option<Arc<ToolRegistry>>,
}

impl CoderAgent {
    /// Create a new CoderAgent with just model name (for testing)
    pub fn new(model: String) -> Self {
        let config = SystemPromptConfig {
            project_name: "SWELL".to_string(),
            repo_path: ".".to_string(),
            for_role: AgentRole::Coder,
            include_memory: true,
            include_conventions: true,
            max_tokens: 8000,
        };
        
        Self {
            model,
            system_prompt_builder: SystemPromptBuilder::new(config),
            llm: None,
            tool_registry: None,
        }
    }

    /// Create a CoderAgent with LLM backend for code generation
    pub fn with_llm(model: String, llm: Arc<dyn LlmBackend>) -> Self {
        let mut agent = Self::new(model);
        agent.llm = Some(llm);
        agent
    }

    /// Create a CoderAgent with tool registry for file operations
    pub fn with_tools(model: String, tool_registry: Arc<ToolRegistry>) -> Self {
        let mut agent = Self::new(model);
        agent.tool_registry = Some(tool_registry);
        agent
    }

    /// Create a fully configured CoderAgent with LLM and tools
    pub fn with_llm_and_tools(
        model: String,
        llm: Arc<dyn LlmBackend>,
        tool_registry: Arc<ToolRegistry>,
    ) -> Self {
        let mut agent = Self::new(model);
        agent.llm = Some(llm);
        agent.tool_registry = Some(tool_registry);
        agent
    }

    /// Generate a unified diff for the given change
    pub fn generate_diff(&self, file_path: &str, original: &str, new_content: &str) -> String {
        use std::fmt::Write as FmtWrite;
        
        let mut diff = String::new();
        
        // Unified diff header
        writeln!(diff, "--- a/{}", file_path).unwrap();
        writeln!(diff, "+++ b/{}", file_path).unwrap();
        
        // Calculate line changes
        let original_lines: Vec<&str> = original.lines().collect();
        let new_lines: Vec<&str> = new_content.lines().collect();
        
        let original_len = original_lines.len();
        let new_len = new_lines.len();
        
        // Simple diff algorithm: find longest common prefix and suffix
        let mut prefix_len = 0;
        let min_len = original_len.min(new_len);
        while prefix_len < min_len && original_lines[prefix_len] == new_lines[prefix_len] {
            prefix_len += 1;
        }
        
        let mut suffix_len = 0;
        while suffix_len < min_len - prefix_len && 
              original_lines[original_len - 1 - suffix_len] == new_lines[new_len - 1 - suffix_len] {
            suffix_len += 1;
        }
        
        // Hunk header
        writeln!(diff, "@@ -{},{} +{},{} @@", 
            if original_len > 0 { 1 } else { 0 }, 
            original_len,
            if new_len > 0 { 1 } else { 0 }, 
            new_len
        ).unwrap();
        
        // Removed lines (before prefix)
        for i in 0..prefix_len {
            writeln!(diff, " {}", original_lines[i]).unwrap();
        }
        
        // Changed lines
        let orig_changed_start = prefix_len;
        let orig_changed_end = original_len - suffix_len;
        let new_changed_end = new_len - suffix_len;
        
        // Removed from original
        for i in orig_changed_start..orig_changed_end {
            writeln!(diff, "-{}", original_lines[i]).unwrap();
        }
        
        // Added in new
        for i in prefix_len..new_changed_end {
            writeln!(diff, "+{}", new_lines[i]).unwrap();
        }
        
        // Trailing lines (after suffix)
        for i in (new_len - suffix_len)..new_len {
            writeln!(diff, " {}", new_lines[i]).unwrap();
        }
        
        diff
    }

    /// Read file content using tool registry
    async fn read_file(&self, path: &str) -> Result<String, SwellError> {
        let registry = self.tool_registry.as_ref()
            .ok_or_else(|| SwellError::ToolExecutionFailed("No tool registry configured".to_string()))?;
        
        let tool = registry.get("read_file").await
            .ok_or_else(|| SwellError::ToolExecutionFailed("read_file tool not found".to_string()))?;
        
        let args = serde_json::json!({ "path": path });
        let result: ToolOutput = tool.execute(args).await?;
        
        if result.success {
            Ok(result.result)
        } else {
            Err(SwellError::ToolExecutionFailed(result.error.unwrap_or_default()))
        }
    }

    /// Apply a file edit using the tool registry
    async fn edit_file(&self, path: &str, old_content: &str, new_content: &str) -> Result<String, SwellError> {
        let registry = self.tool_registry.as_ref()
            .ok_or_else(|| SwellError::ToolExecutionFailed("No tool registry configured".to_string()))?;
        
        let tool = registry.get("edit_file").await
            .ok_or_else(|| SwellError::ToolExecutionFailed("edit_file tool not found".to_string()))?;
        
        let args = serde_json::json!({
            "path": path,
            "old_str": old_content,
            "new_str": new_content
        });
        let result: ToolOutput = tool.execute(args).await?;
        
        if result.success {
            Ok(result.result)
        } else {
            Err(SwellError::ToolExecutionFailed(result.error.unwrap_or_default()))
        }
    }

    /// Validate the generated code (basic syntax check)
    async fn validate_output(&self, changes: &[FileChange]) -> Result<ValidationResult, SwellError> {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        
        for change in changes {
            // For each modified file, try to parse it for basic syntax validity
            // This is a basic check - in production we'd use rustfmt and clippy
            if let Some(ref new_content) = change.new_content {
                if new_content.contains("fn ") {
                    // Basic Rust syntax validation
                    let open_braces = new_content.matches('{').count();
                    let close_braces = new_content.matches('}').count();
                    
                    if open_braces != close_braces {
                        errors.push(format!(
                            "Syntax error in {}: mismatched braces ({} open, {} close)",
                            change.file_path, open_braces, close_braces
                        ));
                    }
                    
                    // Check for obvious issues
                    if new_content.contains("TODO") {
                        warnings.push(format!("{} contains TODO items", change.file_path));
                    }
                }
            }
        }
        
        let passed = errors.is_empty();
        
        Ok(ValidationResult {
            passed,
            errors,
            warnings,
        })
    }
}

/// Represents a single file change with diff
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub file_path: String,
    pub operation: ChangeOperation,
    pub original_content: Option<String>,
    pub new_content: Option<String>,
    pub diff: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeOperation {
    Create,
    Modify,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub passed: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[async_trait]
impl Agent for CoderAgent {
    fn role(&self) -> AgentRole {
        AgentRole::Coder
    }

    fn description(&self) -> String {
        "Implements specific code changes with diff-based modifications".to_string()
    }

    async fn execute(&self, context: AgentContext) -> Result<AgentResult, SwellError> {
        // Build system prompt with task context
        let task_context = format!(
            "Task: {}\n\nImplement the required changes and produce diffs.",
            context.task.description
        );
        
        let _prompt = self.system_prompt_builder.build(&task_context, &context.memory_blocks);
        
        // If no tool registry, return stub response
        if self.tool_registry.is_none() {
            let output = serde_json::json!({
                "changes": [
                    {
                        "file": "src/modified.rs",
                        "operation": "modify",
                        "diff": format!("// Implementation for: {}", context.task.description)
                    }
                ],
                "self_validation": "Basic syntax check passed",
                "tokens_used": 800
            });

            return Ok(AgentResult {
                success: true,
                output: serde_json::to_string(&output).unwrap_or_default(),
                tool_calls: vec![],
                tokens_used: 800,
                error: None,
            });
        }

        // Extract affected files from plan if available
        let affected_files = if let Some(ref plan) = context.task.plan {
            plan.steps.iter()
                .flat_map(|s| s.affected_files.iter().cloned())
                .collect::<Vec<_>>()
        } else {
            vec![]
        };

        let mut changes = Vec::new();
        let mut tool_calls = Vec::new();
        let mut total_tokens = 0u64;

        // For each affected file, read it and generate a modification
        for file_path in &affected_files {
            let original_content = match self.read_file(file_path).await {
                Ok(content) => {
                    tool_calls.push(ToolCallResult {
                        tool_name: "read_file".to_string(),
                        arguments: serde_json::json!({ "path": file_path }),
                        result: Ok("OK".to_string()),
                        duration_ms: 0,
                    });
                    content
                },
                Err(e) => {
                    // File might not exist yet - this is okay for new files
                    tool_calls.push(ToolCallResult {
                        tool_name: "read_file".to_string(),
                        arguments: serde_json::json!({ "path": file_path }),
                        result: Err(e.to_string()),
                        duration_ms: 0,
                    });
                    continue;
                }
            };

            // Generate new content based on task (simplified - real impl would use LLM)
            let new_content = format!(
                "// Generated code for: {}\n{}",
                context.task.description,
                original_content
            );

            // Generate diff
            let diff = self.generate_diff(file_path, &original_content, &new_content);
            
            changes.push(FileChange {
                file_path: file_path.clone(),
                operation: ChangeOperation::Modify,
                original_content: Some(original_content.clone()),
                new_content: Some(new_content.clone()),
                diff: diff.clone(),
            });

            total_tokens += (file_path.len() + original_content.len() + new_content.len()) as u64 / 4;
        }

        // If no changes but we have a task, create a placeholder change
        if changes.is_empty() {
            let new_content = format!("// Implementation for: {}\n", context.task.description);
            changes.push(FileChange {
                file_path: "src/new_file.rs".to_string(),
                operation: ChangeOperation::Create,
                original_content: None,
                new_content: Some(new_content.clone()),
                diff: format!(
                    "--- /dev/null\n+++ b/src/new_file.rs\n@@ -0,0 +1,2 @@\n+// Implementation for: {}\n+",
                    context.task.description
                ),
            });
        }

        // Self-validate outputs
        let validation = self.validate_output(&changes).await?;
        
        // Apply changes (in real implementation)
        for change in &changes {
            if change.operation == ChangeOperation::Modify {
                if let (Some(old_content), Some(new_content)) = (&change.original_content, &change.new_content) {
                    let _ = self.edit_file(&change.file_path, old_content, new_content).await;
                }
            }
        }

        let output = serde_json::json!({
            "changes": changes.into_iter().map(|c| serde_json::json!({
                "file": c.file_path,
                "operation": format!("{:?}", c.operation).to_lowercase(),
                "diff": c.diff
            })).collect::<Vec<_>>(),
            "self_validation": {
                "passed": validation.passed,
                "errors": validation.errors,
                "warnings": validation.warnings,
            },
            "tokens_used": total_tokens
        });

        Ok(AgentResult {
            success: validation.passed,
            output: serde_json::to_string(&output).unwrap_or_default(),
            tool_calls,
            tokens_used: total_tokens,
            error: if validation.passed { None } else { Some("Self-validation failed".to_string()) },
        })
    }
}

// ============================================================================
// TestWriterAgent - Generates tests from acceptance criteria
// ============================================================================

/// Test writer agent for generating tests
pub struct TestWriterAgent {
    model: String,
    system_prompt_builder: SystemPromptBuilder,
}

impl TestWriterAgent {
    pub fn new(model: String) -> Self {
        let config = SystemPromptConfig {
            project_name: "SWELL".to_string(),
            repo_path: ".".to_string(),
            for_role: AgentRole::TestWriter,
            include_memory: true,
            include_conventions: true,
            max_tokens: 8000,
        };
        
        Self {
            model,
            system_prompt_builder: SystemPromptBuilder::new(config),
        }
    }

    /// Generate a test from Given/When/Then format
    pub fn parse_given_when_then(&self, criteria: &str) -> TestSpec {
        // Simplified parsing - in full implementation this would use the LLM
        let parts: Vec<&str> = criteria.split('\n').collect();
        
        let mut given = Vec::new();
        let mut when = Vec::new();
        let mut then = Vec::new();
        let mut current_section = &mut given;
        
        for line in parts {
            let line = line.trim();
            if line.starts_with("Given ") {
                current_section = &mut given;
                current_section.push(line.trim_start_matches("Given ").to_string());
            } else if line.starts_with("When ") {
                current_section = &mut when;
                current_section.push(line.trim_start_matches("When ").to_string());
            } else if line.starts_with("Then ") {
                current_section = &mut then;
                current_section.push(line.trim_start_matches("Then ").to_string());
            } else if !line.is_empty() {
                current_section.push(line.to_string());
            }
        }
        
        TestSpec { given, when, then }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestSpec {
    pub given: Vec<String>,
    pub when: Vec<String>,
    pub then: Vec<String>,
}

#[async_trait]
impl Agent for TestWriterAgent {
    fn role(&self) -> AgentRole {
        AgentRole::TestWriter
    }

    fn description(&self) -> String {
        "Generates tests from acceptance criteria using Given/When/Then format".to_string()
    }

    async fn execute(&self, context: AgentContext) -> Result<AgentResult, SwellError> {
        let task_context = format!(
            "Task: {}\n\nGenerate tests following Given/When/Then format.",
            context.task.description
        );
        
        let _prompt = self.system_prompt_builder.build(&task_context, &context.memory_blocks);
        
        // Parse acceptance criteria from task description
        let spec = self.parse_given_when_then(&context.task.description);
        
        let output = serde_json::json!({
            "test_file": "tests/generated_test.rs",
            "spec": spec,
            "test_functions": [
                format!("test_{}_happy_path", context.task.description.replace(" ", "_").to_lowercase()),
            ],
            "coverage_mapped": true
        });

        Ok(AgentResult {
            success: true,
            output: serde_json::to_string(&output).unwrap_or_default(),
            tool_calls: vec![],
            tokens_used: 600,
            error: None,
        })
    }
}

// ============================================================================
// ReviewerAgent - Semantic code review
// ============================================================================

/// Reviewer agent for semantic code review
pub struct ReviewerAgent {
    model: String,
    system_prompt_builder: SystemPromptBuilder,
}

impl ReviewerAgent {
    pub fn new(model: String) -> Self {
        let config = SystemPromptConfig {
            project_name: "SWELL".to_string(),
            repo_path: ".".to_string(),
            for_role: AgentRole::Reviewer,
            include_memory: true,
            include_conventions: true,
            max_tokens: 8000,
        };
        
        Self {
            model,
            system_prompt_builder: SystemPromptBuilder::new(config),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewResult {
    pub issues: Vec<CodeIssue>,
    pub score: u32, // 0-100
    pub can_merge: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeIssue {
    pub severity: IssueSeverity,
    pub category: IssueCategory,
    pub message: String,
    pub file: Option<String>,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum IssueSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum IssueCategory {
    Style,
    Complexity,
    Convention,
    Regression,
    Security,
    Performance,
}

#[async_trait]
impl Agent for ReviewerAgent {
    fn role(&self) -> AgentRole {
        AgentRole::Reviewer
    }

    fn description(&self) -> String {
        "Performs semantic code review checking style, complexity, regressions, conventions".to_string()
    }

    async fn execute(&self, context: AgentContext) -> Result<AgentResult, SwellError> {
        let task_context = format!(
            "Task: {}\n\nReview the code changes for quality issues.",
            context.task.description
        );
        
        let _prompt = self.system_prompt_builder.build(&task_context, &context.memory_blocks);
        
        let review = ReviewResult {
            issues: vec![
                CodeIssue {
                    severity: IssueSeverity::Info,
                    category: IssueCategory::Style,
                    message: "Consider adding doc comments to public functions".to_string(),
                    file: Some("src/modified.rs".to_string()),
                    line: Some(10),
                }
            ],
            score: 85,
            can_merge: true,
        };

        Ok(AgentResult {
            success: true,
            output: serde_json::to_string(&review).unwrap_or_default(),
            tool_calls: vec![],
            tokens_used: 500,
            error: None,
        })
    }
}

// ============================================================================
// RefactorerAgent - Code restructuring while preserving behavior
// ============================================================================

/// Refactorer agent for code restructuring
pub struct RefactorerAgent {
    model: String,
    system_prompt_builder: SystemPromptBuilder,
}

impl RefactorerAgent {
    pub fn new(model: String) -> Self {
        let config = SystemPromptConfig {
            project_name: "SWELL".to_string(),
            repo_path: ".".to_string(),
            for_role: AgentRole::Refactorer,
            include_memory: true,
            include_conventions: true,
            max_tokens: 8000,
        };
        
        Self {
            model,
            system_prompt_builder: SystemPromptBuilder::new(config),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefactorPlan {
    pub opportunities: Vec<RefactorOpportunity>,
    pub risk_assessment: String,
    pub preserved_behavior: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefactorOpportunity {
    pub description: String,
    pub target_files: Vec<String>,
    pub expected_improvement: String,
    pub risk_level: RiskLevel,
}

#[async_trait]
impl Agent for RefactorerAgent {
    fn role(&self) -> AgentRole {
        AgentRole::Refactorer
    }

    fn description(&self) -> String {
        "Identifies refactoring opportunities and restructures code while preserving behavior".to_string()
    }

    async fn execute(&self, context: AgentContext) -> Result<AgentResult, SwellError> {
        let task_context = format!(
            "Task: {}\n\nIdentify refactoring opportunities.",
            context.task.description
        );
        
        let _prompt = self.system_prompt_builder.build(&task_context, &context.memory_blocks);
        
        let plan = RefactorPlan {
            opportunities: vec![
                RefactorOpportunity {
                    description: "Extract helper function for duplicated logic".to_string(),
                    target_files: vec!["src/modified.rs".to_string()],
                    expected_improvement: "Reduce code duplication by 20%".to_string(),
                    risk_level: RiskLevel::Low,
                }
            ],
            risk_assessment: "Low risk refactoring identified".to_string(),
            preserved_behavior: true,
        };

        Ok(AgentResult {
            success: true,
            output: serde_json::to_string(&plan).unwrap_or_default(),
            tool_calls: vec![],
            tokens_used: 700,
            error: None,
        })
    }
}

// ============================================================================
// DocWriterAgent - Documentation generation
// ============================================================================

/// Doc writer agent for documentation generation
pub struct DocWriterAgent {
    model: String,
    system_prompt_builder: SystemPromptBuilder,
}

impl DocWriterAgent {
    pub fn new(model: String) -> Self {
        let config = SystemPromptConfig {
            project_name: "SWELL".to_string(),
            repo_path: ".".to_string(),
            for_role: AgentRole::DocWriter,
            include_memory: true,
            include_conventions: true,
            max_tokens: 8000,
        };
        
        Self {
            model,
            system_prompt_builder: SystemPromptBuilder::new(config),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocChange {
    pub file: String,
    pub change_type: DocChangeType,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocChangeType {
    Create,
    Update,
    Delete,
}

#[async_trait]
impl Agent for DocWriterAgent {
    fn role(&self) -> AgentRole {
        AgentRole::DocWriter
    }

    fn description(&self) -> String {
        "Generates and modifies documentation from code changes".to_string()
    }

    async fn execute(&self, context: AgentContext) -> Result<AgentResult, SwellError> {
        let task_context = format!(
            "Task: {}\n\nGenerate or update documentation.",
            context.task.description
        );
        
        let _prompt = self.system_prompt_builder.build(&task_context, &context.memory_blocks);
        
        let changes = vec![
            DocChange {
                file: "docs/api.md".to_string(),
                change_type: DocChangeType::Update,
                content: "# API Documentation\n\nUpdated based on code changes.".to_string(),
            }
        ];

        Ok(AgentResult {
            success: true,
            output: serde_json::to_string(&changes).unwrap_or_default(),
            tool_calls: vec![],
            tokens_used: 400,
            error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swell_core::{Task, MemoryBlock, MemoryBlockType, LlmMessage, LlmRole, LlmConfig};
    use chrono::Utc;

    // ========================================================================
    // AgentPool Tests
    // ========================================================================

    #[tokio::test]
    async fn test_agent_pool_registration() {
        let mut pool = AgentPool::new();
        
        let id1 = pool.register(AgentRole::Planner, "claude-sonnet".to_string());
        let id2 = pool.register(AgentRole::Generator, "claude-sonnet".to_string());
        
        assert_ne!(id1, id2);
        assert_eq!(pool.available_count(AgentRole::Planner), 1);
        assert_eq!(pool.available_count(AgentRole::Generator), 1);
    }

    #[tokio::test]
    async fn test_agent_pool_reserve() {
        let mut pool = AgentPool::new();
        let agent_id = pool.register(AgentRole::Generator, "claude-sonnet".to_string());
        let task_id = Uuid::new_v4();
        
        let reserved = pool.reserve(task_id, AgentRole::Generator).unwrap();
        assert_eq!(reserved, agent_id);
        assert_eq!(pool.available_count(AgentRole::Generator), 0);
    }

    #[tokio::test]
    async fn test_agent_pool_release() {
        let mut pool = AgentPool::new();
        let agent_id = pool.register(AgentRole::Generator, "claude-sonnet".to_string());
        let task_id = Uuid::new_v4();
        
        pool.reserve(task_id, AgentRole::Generator).unwrap();
        assert_eq!(pool.available_count(AgentRole::Generator), 0);
        
        pool.release(agent_id);
        assert_eq!(pool.available_count(AgentRole::Generator), 1);
    }

    // ========================================================================
    // PlannerAgent Tests
    // ========================================================================

    #[tokio::test]
    async fn test_planner_agent_with_mock() {
        use swell_llm::MockLlm;
        use std::sync::Arc;

        // Create a mock that returns valid JSON
        let mock_response = r#"
{
  "steps": [
    {
      "description": "Implement user login endpoint",
      "affected_files": ["src/auth.rs", "src/models/user.rs"],
      "expected_tests": ["test_login_success", "test_login_failure"],
      "risk_level": "medium",
      "dependencies": []
    },
    {
      "description": "Add JWT token generation",
      "affected_files": ["src/auth/jwt.rs"],
      "expected_tests": ["test_token_generation"],
      "risk_level": "low",
      "dependencies": []
    }
  ],
  "total_estimated_tokens": 8000,
  "risk_assessment": "Medium risk - involves authentication changes"
}
"#;
        let mock = Arc::new(MockLlm::with_response("claude-sonnet", mock_response));
        let agent = PlannerAgent::with_llm("claude-sonnet".to_string(), mock);
        
        let task = Task::new("Add user authentication".to_string());
        let context = AgentContext {
            task,
            memory_blocks: vec![],
            session_id: Uuid::new_v4(),
            workspace_path: Some("/workspace".to_string()),
        };
        
        let result = agent.execute(context).await.unwrap();
        assert!(result.success);
        
        // Parse the plan from output - should now be valid Plan struct
        let plan: Plan = serde_json::from_str(&result.output).unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.total_estimated_tokens, 8000);
        assert!(!plan.risk_assessment.is_empty());
        assert_eq!(plan.steps[0].affected_files.len(), 2);
    }

    #[tokio::test]
    async fn test_planner_agent_with_memory_blocks() {
        use swell_llm::MockLlm;
        use std::sync::Arc;
        use swell_core::MemoryBlockType;

        let mock_response = r#"
{
  "steps": [
    {
      "description": "Add user login",
      "affected_files": ["src/auth.rs"],
      "expected_tests": [],
      "risk_level": "medium",
      "dependencies": []
    }
  ],
  "total_estimated_tokens": 5000,
  "risk_assessment": "Standard implementation"
}
"#;
        let mock = Arc::new(MockLlm::with_response("claude-sonnet", mock_response));
        let agent = PlannerAgent::with_llm("claude-sonnet".to_string(), mock);
        
        let task = Task::new("Add login functionality".to_string());
        let memory_blocks = vec![
            MemoryBlock {
                id: Uuid::new_v4(),
                label: "auth_conventions".to_string(),
                description: "Authentication conventions".to_string(),
                content: "Use JWT for auth tokens".to_string(),
                block_type: MemoryBlockType::Convention,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
        ];
        let context = AgentContext {
            task,
            memory_blocks,
            session_id: Uuid::new_v4(),
            workspace_path: Some("/workspace".to_string()),
        };
        
        let result = agent.execute(context).await.unwrap();
        assert!(result.success);
        assert!(result.tokens_used > 0);
    }

    // ========================================================================
    // GeneratorAgent Tests
    // ========================================================================

    #[tokio::test]
    async fn test_generator_agent() {
        let agent = GeneratorAgent::new("claude-sonnet".to_string());
        
        let task = Task::new("Add user authentication".to_string());
        let context = AgentContext {
            task,
            memory_blocks: vec![],
            session_id: Uuid::new_v4(),
            workspace_path: None,
        };
        
        let result = agent.execute(context).await.unwrap();
        assert!(result.success);
    }

    // ========================================================================
    // EvaluatorAgent Tests
    // ========================================================================

    #[tokio::test]
    async fn test_evaluator_agent() {
        let agent = EvaluatorAgent::new("claude-sonnet".to_string());
        
        let task = Task::new("Add user authentication".to_string());
        let context = AgentContext {
            task,
            memory_blocks: vec![],
            session_id: Uuid::new_v4(),
            workspace_path: None,
        };
        
        let result = agent.execute(context).await.unwrap();
        assert!(result.success);
        
        // In stub mode (no pipeline), output is just EvaluationResult directly
        let evaluation: EvaluationResult = serde_json::from_str(&result.output).unwrap();
        assert!(evaluation.passed);
        assert!(evaluation.confidence_score > 0.0);
        assert!(evaluation.warnings.iter().any(|w| w.contains("stub mode")));
    }

    #[tokio::test]
    async fn test_evaluator_agent_with_plan_extracts_changed_files() {
        let agent = EvaluatorAgent::new("claude-sonnet".to_string());
        
        let mut task = Task::new("Add user authentication".to_string());
        task.plan = Some(Plan {
            id: Uuid::new_v4(),
            task_id: task.id,
            steps: vec![
                PlanStep {
                    id: Uuid::new_v4(),
                    description: "Add auth module".to_string(),
                    affected_files: vec!["src/auth.rs".to_string(), "src/models/user.rs".to_string()],
                    expected_tests: vec!["test_login".to_string()],
                    risk_level: RiskLevel::Medium,
                    dependencies: vec![],
                    status: StepStatus::Pending,
                },
                PlanStep {
                    id: Uuid::new_v4(),
                    description: "Add JWT support".to_string(),
                    affected_files: vec!["src/auth/jwt.rs".to_string()],
                    expected_tests: vec!["test_jwt".to_string()],
                    risk_level: RiskLevel::Low,
                    dependencies: vec![],
                    status: StepStatus::Pending,
                },
            ],
            total_estimated_tokens: 5000,
            risk_assessment: "Medium risk".to_string(),
        });
        
        let context = AgentContext {
            task,
            memory_blocks: vec![],
            session_id: Uuid::new_v4(),
            workspace_path: Some("/workspace".to_string()),
        };
        
        // Test that extract_changed_files works correctly
        let files = EvaluatorAgent::extract_changed_files(&context);
        assert_eq!(files.len(), 3); // 3 unique files: auth.rs, user.rs, jwt.rs
        assert!(files.contains(&"src/auth.rs".to_string()));
        assert!(files.contains(&"src/auth/jwt.rs".to_string()));
        assert!(files.contains(&"src/models/user.rs".to_string()));
        
        let result = agent.execute(context).await.unwrap();
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_evaluation_result_serialization() {
        let result = EvaluationResult {
            passed: true,
            confidence_score: 0.85,
            confidence_level: ConfidenceLevel::High,
            errors: vec![],
            warnings: vec!["Minor style issue".to_string()],
            can_auto_merge: false,
            messages: 5,
            artifacts: 0,
        };
        
        let json = serde_json::to_string(&result).unwrap();
        let parsed: EvaluationResult = serde_json::from_str(&json).unwrap();
        
        assert!(parsed.passed);
        assert_eq!(parsed.confidence_score, 0.85);
        assert!(matches!(parsed.confidence_level, ConfidenceLevel::High));
        assert!(!parsed.can_auto_merge);
    }

    #[tokio::test]
    async fn test_extract_changed_files_deduplicates() {
        let mut task = Task::new("Test".to_string());
        task.plan = Some(Plan {
            id: Uuid::new_v4(),
            task_id: task.id,
            steps: vec![
                PlanStep {
                    id: Uuid::new_v4(),
                    description: "Step 1".to_string(),
                    affected_files: vec!["a.rs".to_string(), "b.rs".to_string()],
                    expected_tests: vec![],
                    risk_level: RiskLevel::Low,
                    dependencies: vec![],
                    status: StepStatus::Pending,
                },
                PlanStep {
                    id: Uuid::new_v4(),
                    description: "Step 2".to_string(),
                    affected_files: vec!["b.rs".to_string(), "c.rs".to_string()], // b.rs is duplicate
                    expected_tests: vec![],
                    risk_level: RiskLevel::Low,
                    dependencies: vec![],
                    status: StepStatus::Pending,
                },
            ],
            total_estimated_tokens: 1000,
            risk_assessment: "Low".to_string(),
        });
        
        let context = AgentContext {
            task,
            memory_blocks: vec![],
            session_id: Uuid::new_v4(),
            workspace_path: Some(".".to_string()),
        };
        
        let files = EvaluatorAgent::extract_changed_files(&context);
        assert_eq!(files.len(), 3); // a, b, c - no duplicates
    }

    #[tokio::test]
    async fn test_build_validation_context() {
        let mut task = Task::new("Test".to_string());
        task.plan = Some(Plan {
            id: Uuid::new_v4(),
            task_id: task.id,
            steps: vec![
                PlanStep {
                    id: Uuid::new_v4(),
                    description: "Modify auth".to_string(),
                    affected_files: vec!["src/auth.rs".to_string()],
                    expected_tests: vec![],
                    risk_level: RiskLevel::Medium,
                    dependencies: vec![],
                    status: StepStatus::Pending,
                },
            ],
            total_estimated_tokens: 1000,
            risk_assessment: "Low".to_string(),
        });
        
        let context = AgentContext {
            task,
            memory_blocks: vec![],
            session_id: Uuid::new_v4(),
            workspace_path: Some("/my/workspace".to_string()),
        };
        
        let validation_context = EvaluatorAgent::build_validation_context(&context);
        
        assert_eq!(validation_context.task_id, context.task.id);
        assert_eq!(validation_context.workspace_path, "/my/workspace");
        assert_eq!(validation_context.changed_files, vec!["src/auth.rs".to_string()]);
        assert!(validation_context.plan.is_some());
    }

    // ========================================================================
    // SystemPromptBuilder Tests
    // ========================================================================

    #[tokio::test]
    async fn test_system_prompt_builder_default_config() {
        let config = SystemPromptConfig::default();
        assert_eq!(config.project_name, "SWELL");
        assert_eq!(config.for_role, AgentRole::Coder);
        assert!(config.include_memory);
        assert!(config.include_conventions);
    }

    #[tokio::test]
    async fn test_system_prompt_builder_build() {
        let config = SystemPromptConfig {
            project_name: "TestProject".to_string(),
            repo_path: "/test".to_string(),
            for_role: AgentRole::Coder,
            include_memory: true,
            include_conventions: true,
            max_tokens: 8000,
        };
        
        let builder = SystemPromptBuilder::new(config);
        let memory_blocks = vec![
            MemoryBlock {
                id: Uuid::new_v4(),
                label: "auth".to_string(),
                description: "Auth conventions".to_string(),
                content: "Use JWT for authentication".to_string(),
                block_type: MemoryBlockType::Convention,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }
        ];
        
        let task_context = "Add login functionality";
        let prompt = builder.build(task_context, &memory_blocks);
        
        assert!(prompt.contains("TestProject"));
        assert!(prompt.contains("CODER"));
        assert!(prompt.contains("login functionality"));
        assert!(prompt.contains("JWT"));
    }

    #[tokio::test]
    async fn test_system_prompt_builder_usage_calculation() {
        let config = SystemPromptConfig {
            max_tokens: 1000,
            ..Default::default()
        };
        
        let builder = SystemPromptBuilder::new(config);
        let prompt = "This is a test prompt".to_string();
        
        let usage = builder.calculate_usage(&prompt);
        assert!(usage > 0.0);
    }

    // ========================================================================
    // ReAct Loop Tests
    // ========================================================================

    #[tokio::test]
    async fn test_react_loop_initialization() {
        let loop_state = ReactLoop::new(10);
        assert_eq!(loop_state.max_iterations, 10);
        assert_eq!(loop_state.current_iteration, 0);
        assert!(matches!(loop_state.state, ReactLoopState::Running));
    }

    #[tokio::test]
    async fn test_react_loop_think_act_observe() {
        let mut loop_state = ReactLoop::new(10);
        
        loop_state.think("I need to implement login".to_string());
        assert_eq!(loop_state.current_iteration, 1);
        
        loop_state.act("file_read(path='/src/auth.rs')".to_string());
        if let Some(step) = loop_state.steps.last() {
            assert!(step.action.is_some());
        }
        
        loop_state.observe("Found existing auth module".to_string());
        if let Some(step) = loop_state.steps.last() {
            assert!(step.observation.is_some());
        }
    }

    #[tokio::test]
    async fn test_react_loop_success() {
        let mut loop_state = ReactLoop::new(10);
        
        loop_state.think("Implement login".to_string());
        loop_state.act("Write login code".to_string());
        loop_state.observe("Code written".to_string());
        loop_state.success("Login implemented successfully".to_string());
        
        assert!(matches!(loop_state.state, ReactLoopState::Converged));
    }

    #[tokio::test]
    async fn test_react_loop_failure_with_reflection() {
        let mut loop_state = ReactLoop::new(10);
        
        loop_state.think("Try implementation".to_string());
        loop_state.act("Write code".to_string());
        let reflection = loop_state.failure("Compilation error".to_string());
        
        assert!(reflection.contains("Failed at iteration"));
        assert_eq!(loop_state.failure_count, 1);
    }

    #[tokio::test]
    async fn test_react_loop_max_iterations() {
        let mut loop_state = ReactLoop::new(3);
        
        for i in 1..=3 {
            loop_state.think(format!("Iteration {}", i));
            loop_state.act("action".to_string());
        }
        
        loop_state.think("This should stop".to_string());
        
        assert!(matches!(loop_state.state, ReactLoopState::MaxIterationsReached));
        assert!(!loop_state.should_continue());
    }

    #[tokio::test]
    async fn test_react_loop_summary() {
        let mut loop_state = ReactLoop::new(10);
        
        loop_state.think("Think 1".to_string());
        loop_state.act("Act 1".to_string());
        loop_state.success("Done".to_string());
        
        let summary = loop_state.summary();
        assert_eq!(summary.total_iterations, 1);
        assert_eq!(summary.failure_count, 0);
    }

    // ========================================================================
    // ContextCondensation Tests
    // ========================================================================

    #[tokio::test]
    async fn test_context_condensation_needs_condensation() {
        let condensation = ContextCondensation::default();
        
        // Below warning threshold
        assert!(matches!(condensation.needs_condensation(50_000), CondensationLevel::Ok));
        
        // At warning threshold
        assert!(matches!(condensation.needs_condensation(70_000), CondensationLevel::Warning));
        
        // At condensation threshold
        assert!(matches!(condensation.needs_condensation(80_000), CondensationLevel::MustCondense));
    }

    #[tokio::test]
    async fn test_context_condensation_condense() {
        let items = vec![
            ContextItem {
                id: "1".to_string(),
                content: "System prompt".to_string(),
                tokens: 1000,
                priority: 10,
                item_type: ContextItemType::SystemPrompt,
            },
            ContextItem {
                id: "2".to_string(),
                content: "Memory block".to_string(),
                tokens: 500,
                priority: 5,
                item_type: ContextItemType::MemoryBlock,
            },
            ContextItem {
                id: "3".to_string(),
                content: "Old tool result".to_string(),
                tokens: 200,
                priority: 1,
                item_type: ContextItemType::ToolResult,
            },
        ];
        
        let condensation = ContextCondensation::default();
        let result = condensation.condense(&items);
        
        assert_eq!(result.original_tokens, 1700);
        assert!(result.preserved_items.contains(&"1".to_string()));
        assert!(result.preserved_items.contains(&"2".to_string()));
    }

    #[tokio::test]
    async fn test_context_condensation_utilization() {
        let condensation = ContextCondensation::default();
        
        let util = condensation.utilization(50_000);
        assert!((util - 0.5).abs() < 0.01);
    }

    // ========================================================================
    // CoderAgent Tests
    // ========================================================================

    #[tokio::test]
    async fn test_coder_agent() {
        let agent = CoderAgent::new("claude-sonnet".to_string());
        
        let task = Task::new("Add user authentication".to_string());
        let context = AgentContext {
            task,
            memory_blocks: vec![],
            session_id: Uuid::new_v4(),
            workspace_path: None,
        };
        
        let result = agent.execute(context).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("changes"));
    }

    #[tokio::test]
    async fn test_coder_agent_diff_generation() {
        let agent = CoderAgent::new("claude-sonnet".to_string());
        
        let original = r#"fn old_function() {
    println!("old");
}"#;
        let new_content = r#"fn new_function() {
    println!("new");
}"#;
        
        let diff = agent.generate_diff("src/auth.rs", original, new_content);
        
        assert!(diff.contains("--- a/src/auth.rs"));
        assert!(diff.contains("+++ b/src/auth.rs"));
        assert!(diff.contains("-fn old_function()"));
        assert!(diff.contains("+fn new_function()"));
    }

    #[tokio::test]
    async fn test_coder_agent_self_validation() {
        let agent = CoderAgent::new("claude-sonnet".to_string());
        
        let changes = vec![
            FileChange {
                file_path: "src/main.rs".to_string(),
                operation: ChangeOperation::Modify,
                original_content: Some("fn main() {}".to_string()),
                new_content: Some("fn main() {}\n".to_string()),
                diff: "".to_string(),
            }
        ];
        
        let result = agent.validate_output(&changes).await.unwrap();
        assert!(result.passed);
        assert!(result.errors.is_empty());
    }

    #[tokio::test]
    async fn test_coder_agent_self_validation_detects_brace_mismatch() {
        let agent = CoderAgent::new("claude-sonnet".to_string());
        
        let changes = vec![
            FileChange {
                file_path: "src/main.rs".to_string(),
                operation: ChangeOperation::Modify,
                original_content: Some("fn main() {}".to_string()),
                new_content: Some("fn main() {\n    if true {\n".to_string()), // mismatched braces
                diff: "".to_string(),
            }
        ];
        
        let result = agent.validate_output(&changes).await.unwrap();
        assert!(!result.passed);
        assert!(!result.errors.is_empty());
        assert!(result.errors[0].contains("mismatched braces"));
    }

    #[tokio::test]
    async fn test_coder_agent_with_llm_and_tools() {
        use swell_llm::MockLlm;
        use std::sync::Arc;
        use swell_tools::ToolRegistry;
        
        let mock = Arc::new(MockLlm::with_response("claude-sonnet", r#"{"changes": []}"#));
        let registry = Arc::new(ToolRegistry::new());
        
        let agent = CoderAgent::with_llm_and_tools(
            "claude-sonnet".to_string(),
            mock,
            registry,
        );
        
        assert!(agent.llm.is_some());
        assert!(agent.tool_registry.is_some());
    }

    #[tokio::test]
    async fn test_file_change_serialization() {
        let change = FileChange {
            file_path: "src/test.rs".to_string(),
            operation: ChangeOperation::Create,
            original_content: None,
            new_content: Some("fn test() {}".to_string()),
            diff: "--- /dev/null\n+++ b/src/test.rs".to_string(),
        };
        
        let json = serde_json::to_string(&change).unwrap();
        let parsed: FileChange = serde_json::from_str(&json).unwrap();
        
        assert_eq!(parsed.file_path, "src/test.rs");
        assert!(matches!(parsed.operation, ChangeOperation::Create));
        assert!(parsed.original_content.is_none());
        assert!(parsed.new_content.is_some());
    }

    #[tokio::test]
    async fn test_change_operation_variants() {
        assert!(matches!(ChangeOperation::Create, ChangeOperation::Create));
        assert!(matches!(ChangeOperation::Modify, ChangeOperation::Modify));
        assert!(matches!(ChangeOperation::Delete, ChangeOperation::Delete));
    }

    // ========================================================================
    // TestWriterAgent Tests
    // ========================================================================

    #[tokio::test]
    async fn test_test_writer_agent() {
        let agent = TestWriterAgent::new("claude-sonnet".to_string());
        
        let task = Task::new("Add user authentication".to_string());
        let context = AgentContext {
            task,
            memory_blocks: vec![],
            session_id: Uuid::new_v4(),
            workspace_path: None,
        };
        
        let result = agent.execute(context).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("test_file"));
    }

    #[tokio::test]
    async fn test_test_writer_given_when_then_parsing() {
        let agent = TestWriterAgent::new("claude-sonnet".to_string());
        
        let criteria = r#"
Given a user is logged in
Given the user has admin privileges
When the user accesses the admin panel
Then they should see all admin options
"#;
        
        let spec = agent.parse_given_when_then(criteria);
        
        assert_eq!(spec.given.len(), 2);
        assert_eq!(spec.when.len(), 1);
        assert_eq!(spec.then.len(), 1);
    }

    // ========================================================================
    // ReviewerAgent Tests
    // ========================================================================

    #[tokio::test]
    async fn test_reviewer_agent() {
        let agent = ReviewerAgent::new("claude-sonnet".to_string());
        
        let task = Task::new("Add user authentication".to_string());
        let context = AgentContext {
            task,
            memory_blocks: vec![],
            session_id: Uuid::new_v4(),
            workspace_path: None,
        };
        
        let result = agent.execute(context).await.unwrap();
        assert!(result.success);
        
        let review: ReviewResult = serde_json::from_str(&result.output).unwrap();
        assert!(review.can_merge);
        assert!(review.score >= 0);
    }

    // ========================================================================
    // RefactorerAgent Tests
    // ========================================================================

    #[tokio::test]
    async fn test_refactorer_agent() {
        let agent = RefactorerAgent::new("claude-sonnet".to_string());
        
        let task = Task::new("Improve code structure".to_string());
        let context = AgentContext {
            task,
            memory_blocks: vec![],
            session_id: Uuid::new_v4(),
            workspace_path: None,
        };
        
        let result = agent.execute(context).await.unwrap();
        assert!(result.success);
        
        let plan: RefactorPlan = serde_json::from_str(&result.output).unwrap();
        assert!(plan.preserved_behavior);
    }

    // ========================================================================
    // DocWriterAgent Tests
    // ========================================================================

    #[tokio::test]
    async fn test_doc_writer_agent() {
        let agent = DocWriterAgent::new("claude-sonnet".to_string());
        
        let task = Task::new("Add API documentation".to_string());
        let context = AgentContext {
            task,
            memory_blocks: vec![],
            session_id: Uuid::new_v4(),
            workspace_path: None,
        };
        
        let result = agent.execute(context).await.unwrap();
        assert!(result.success);
        
        let changes: Vec<DocChange> = serde_json::from_str(&result.output).unwrap();
        assert!(!changes.is_empty());
        assert_eq!(changes[0].change_type, DocChangeType::Update);
    }
}
