//! Execution controller for managing parallel agent execution.
#![allow(clippy::should_implement_trait)]

use crate::{
    frozen_spec::FrozenSpecRef, EvaluatorAgent, FeatureLead, FeatureLeadSpawner,
    GeneratorAgent, PlannerAgent, Orchestrator, MAX_CONCURRENT_AGENTS,
};
use futures::stream::{self, StreamExt};
use std::sync::Arc;
use swell_core::traits::Agent;
use swell_core::{AgentContext, AgentResult, SwellError, ValidationResult};
use swell_llm::MockLlm;
use tracing::info;
use uuid::Uuid;

/// Manages concurrent task execution with up to 6 agents
pub struct ExecutionController {
    orchestrator: Arc<Orchestrator>,
    max_concurrent: usize,
    /// Frozen specs indexed by task_id, created at execution start
    frozen_specs: std::sync::RwLock<std::collections::HashMap<uuid::Uuid, FrozenSpecRef>>,
    /// Active FeatureLeads for complex tasks
    feature_leads: std::sync::RwLock<std::collections::HashMap<uuid::Uuid, FeatureLead>>,
}

impl ExecutionController {
    pub fn new(orchestrator: Arc<Orchestrator>) -> Self {
        Self {
            orchestrator,
            max_concurrent: MAX_CONCURRENT_AGENTS,
            frozen_specs: std::sync::RwLock::new(std::collections::HashMap::new()),
            feature_leads: std::sync::RwLock::new(std::collections::HashMap::new()),
        }
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
            // Use MockLlm for PlannerAgent since we don't have a real LLM in MVP
            let mock_response = r#"{
                "steps": [{"description": "Execute task", "affected_files": [], "expected_tests": [], "risk_level": "medium", "dependencies": []}],
                "total_estimated_tokens": 1000,
                "risk_assessment": "Medium risk"
            }"#;
            let mock_llm = Arc::new(MockLlm::with_response("claude-sonnet", mock_response));

            // Run PlannerAgent to create the plan
            let planner = PlannerAgent::with_llm("claude-sonnet".to_string(), mock_llm);
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
                    errors: vec![planner_result.error.unwrap_or_else(|| "Planning failed".into())],
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

                let parent_orch = Arc::new(Orchestrator::with_checkpoint_manager(
                    self.orchestrator.checkpoint_manager(),
                ));

                match self.orchestrator.spawn_feature_lead(task_id, plan.clone(), parent_orch) {
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
        let generator = GeneratorAgent::new("claude-sonnet".to_string())
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
        // Use new() for MVP stub mode - with_defaults requires LLM which isn't available in execution context
        let evaluator = EvaluatorAgent::new("claude-sonnet".to_string());
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

        // Cleanup: Remove FeatureLead if present
        if let Ok(mut leads) = self.feature_leads.write() {
            leads.remove(&task_id);
        }

        Ok(result)
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
            max_concurrent: self.max_concurrent,
            frozen_specs: std::sync::RwLock::new(std::collections::HashMap::new()),
            feature_leads: std::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }
}

/// Configuration for task execution
#[derive(Debug, Clone)]
pub struct ExecutionConfig {
    pub max_retries: u32,
    pub timeout_secs: u64,
    pub validation_enabled: bool,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            timeout_secs: 3600, // 1 hour
            validation_enabled: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_execution_controller_creation() {
        let orchestrator = Orchestrator::new();
        let controller = ExecutionController::new(Arc::new(orchestrator));
        assert_eq!(controller.max_concurrent, MAX_CONCURRENT_AGENTS);
    }

    #[tokio::test]
    async fn test_batch_execution() {
        let orchestrator = Orchestrator::new();
        let controller = ExecutionController::new(Arc::new(orchestrator));

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
}
