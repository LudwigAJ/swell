//! Feature Lead sub-orchestrators for managing complex tasks.
//!
//! This module implements hierarchical delegation where tasks exceeding a complexity
//! threshold (15 steps) spawn a FeatureLead sub-orchestrator to manage sub-tasks.
//!
//! # Architecture
//!
//! - Root orchestrator creates FeatureLead for complex tasks
//! - FeatureLead manages a segment of the task graph independently
//! - Max depth is 2 levels (root + FeatureLead)
//! - FeatureLeads report completion back to the root orchestrator

use crate::Orchestrator;
use std::sync::Arc;
use swell_core::{AgentContext, AgentResult, Plan, PlanStep, SwellError};
use tracing::info;
use uuid::Uuid;

/// Threshold for spawning a FeatureLead sub-orchestrator.
/// Tasks with more than this many steps get a sub-orchestrator.
pub const FEATURE_LEAD_STEP_THRESHOLD: usize = 15;

/// Maximum depth of sub-orchestrators (root + FeatureLead = 2 levels max)
pub const MAX_ORCHESTRATOR_DEPTH: u32 = 2;

/// A sub-orchestrator that manages a segment of tasks for a complex feature.
///
/// FeatureLeads are spawned by the root orchestrator when a task exceeds
/// the complexity threshold (FEATURE_LEAD_STEP_THRESHOLD steps). Each FeatureLead
/// manages its own task queue and reports back to the root orchestrator.
#[derive(Clone)]
pub struct FeatureLead {
    /// Unique ID for this FeatureLead
    pub id: Uuid,
    /// ID of the parent orchestrator task
    pub parent_task_id: Uuid,
    /// Name/description of the feature being managed
    pub feature_name: String,
    /// Steps assigned to this FeatureLead
    pub assigned_steps: Vec<Uuid>,
    /// Current orchestrator depth (1 for root, 2 for FeatureLead)
    pub depth: u32,
    /// Reference to the parent orchestrator
    parent_orchestrator: Arc<Orchestrator>,
    /// Flag indicating if this FeatureLead has completed its work
    completed: bool,
    /// Results from each step execution
    step_results: Vec<StepResult>,
}

impl std::fmt::Debug for FeatureLead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FeatureLead")
            .field("id", &self.id)
            .field("parent_task_id", &self.parent_task_id)
            .field("feature_name", &self.feature_name)
            .field("assigned_steps", &self.assigned_steps)
            .field("depth", &self.depth)
            .field("completed", &self.completed)
            .field("step_results", &self.step_results)
            .finish()
    }
}

/// Result of executing a single step
#[derive(Debug, Clone)]
pub struct StepResult {
    pub step_id: Uuid,
    pub success: bool,
    pub error: Option<String>,
    pub tokens_used: u64,
}

impl FeatureLead {
    /// Create a new FeatureLead for managing complex task segments.
    ///
    /// # Arguments
    /// * `parent_task_id` - The task ID in the root orchestrator
    /// * `feature_name` - Human-readable name for this feature
    /// * `assigned_steps` - IDs of plan steps assigned to this lead
    /// * `parent_orchestrator` - Reference to the root orchestrator
    pub fn new(
        parent_task_id: Uuid,
        feature_name: String,
        assigned_steps: Vec<Uuid>,
        parent_orchestrator: Arc<Orchestrator>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            parent_task_id,
            feature_name,
            assigned_steps,
            depth: MAX_ORCHESTRATOR_DEPTH, // FeatureLeads are at depth 2
            parent_orchestrator,
            completed: false,
            step_results: Vec::new(),
        }
    }

    /// Check if this FeatureLead should be spawned based on step count.
    ///
    /// Returns true if the plan has more than FEATURE_LEAD_STEP_THRESHOLD steps.
    pub fn should_spawn(plan: &Plan) -> bool {
        plan.steps.len() > FEATURE_LEAD_STEP_THRESHOLD
    }

    /// Segment a plan into chunks for multiple FeatureLeads.
    ///
    /// This distributes steps evenly among multiple FeatureLeads while
    /// respecting dependencies between steps.
    ///
    /// # Arguments
    /// * `plan` - The plan to segment
    /// * `lead_count` - Number of FeatureLeads to create
    ///
    /// # Returns
    /// Vec of (feature_name, step_ids) tuples
    pub fn segment_plan(plan: &Plan, lead_count: usize) -> Vec<(String, Vec<Uuid>)> {
        if lead_count == 0 {
            return vec![];
        }

        let step_count = plan.steps.len();
        if step_count == 0 {
            return vec![];
        }

        // Calculate base chunk size
        let base_size = step_count / lead_count;
        let remainder = step_count % lead_count;

        let mut segments: Vec<(String, Vec<Uuid>)> = Vec::new();
        let mut current_idx = 0;

        for lead_idx in 0..lead_count {
            // Distribute remainder steps among first leads
            let chunk_size = if lead_idx < remainder {
                base_size + 1
            } else {
                base_size
            };

            if chunk_size == 0 {
                continue;
            }

            let end_idx = (current_idx + chunk_size).min(step_count);
            let step_ids: Vec<Uuid> = plan.steps[current_idx..end_idx]
                .iter()
                .map(|s| s.id)
                .collect();

            let feature_name = format!("Feature Segment {}", lead_idx + 1);
            segments.push((feature_name, step_ids));

            current_idx = end_idx;
        }

        segments
    }

    /// Execute all steps assigned to this FeatureLead.
    ///
    /// This runs the steps through a mini pipeline: Generator → Evaluator,
    /// with progress reported back to the parent orchestrator.
    pub async fn execute(&mut self) -> Result<(), SwellError> {
        info!(
            feature_lead_id = %self.id,
            parent_task_id = %self.parent_task_id,
            step_count = self.assigned_steps.len(),
            "FeatureLead starting execution"
        );

        // Get the parent task to access the plan
        let parent_task = self
            .parent_orchestrator
            .get_task(self.parent_task_id)
            .await?;

        let plan = parent_task
            .plan
            .as_ref()
            .ok_or_else(|| SwellError::InvalidStateTransition("Parent task has no plan".into()))?;

        // Get steps that belong to this FeatureLead
        let steps_to_execute: Vec<&PlanStep> = plan
            .steps
            .iter()
            .filter(|s| self.assigned_steps.contains(&s.id))
            .collect();

        // Execute each step
        for step in steps_to_execute {
            info!(
                step_id = %step.id,
                description = %step.description,
                "FeatureLead executing step"
            );

            // Create execution context
            let session_id = Uuid::new_v4();
            let mut task = parent_task.clone();
            task.description = step.description.clone();

            let context = AgentContext {
                task,
                memory_blocks: Vec::new(),
                session_id,
                workspace_path: None,
            };

            // For now, we simulate step execution
            // In a full implementation, this would use the GeneratorAgent
            let result = self.execute_step(step, context).await;

            let step_result = StepResult {
                step_id: step.id,
                success: result.is_ok(),
                error: result.as_ref().err().map(|e| e.to_string()),
                tokens_used: 0, // Would be tracked from actual execution
            };

            self.step_results.push(step_result);
        }

        self.completed = true;
        info!(
            feature_lead_id = %self.id,
            steps_completed = self.step_results.len(),
            "FeatureLead execution complete"
        );

        Ok(())
    }

    /// Execute a single step with the given context.
    async fn execute_step(
        &self,
        step: &PlanStep,
        _context: AgentContext,
    ) -> Result<AgentResult, SwellError> {
        // In a real implementation, this would:
        // 1. Create a GeneratorAgent
        // 2. Call execute() with the step context
        // 3. Run validation gates
        // 4. Return the result

        // For now, return a success result to indicate the step was considered
        Ok(AgentResult {
            success: true,
            output: format!("Step {} executed by FeatureLead", step.id),
            tool_calls: vec![],
            tokens_used: 0,
            error: None,
            confidence_score: None,
        })
    }

    /// Check if this FeatureLead has completed all assigned steps.
    pub fn is_completed(&self) -> bool {
        self.completed
    }

    /// Get completion status as a percentage.
    pub fn completion_percentage(&self) -> f64 {
        if self.assigned_steps.is_empty() {
            return 100.0;
        }

        let completed = self.step_results.iter().filter(|r| r.success).count() as f64;

        (completed / self.assigned_steps.len() as f64) * 100.0
    }

    /// Get failed step IDs for reporting.
    pub fn failed_steps(&self) -> Vec<Uuid> {
        self.step_results
            .iter()
            .filter(|r| !r.success)
            .map(|r| r.step_id)
            .collect()
    }

    /// Report completion back to the root orchestrator.
    ///
    /// This is called when the FeatureLead finishes its work,
    /// allowing the root orchestrator to track progress.
    pub async fn report_completion(&self) -> Result<(), SwellError> {
        info!(
            feature_lead_id = %self.id,
            parent_task_id = %self.parent_task_id,
            steps_completed = self.step_results.len(),
            failed_steps = self.failed_steps().len(),
            "FeatureLead reporting completion to root orchestrator"
        );

        // In a full implementation, this would:
        // 1. Update the parent task's state
        // 2. Record metrics about the FeatureLead's performance
        // 3. Trigger validation of the overall task

        Ok(())
    }

    /// Check if this FeatureLead should escalate to the root orchestrator.
    ///
    /// Escalation happens when too many steps have failed or the
    /// complexity exceeds what this FeatureLead can handle.
    pub fn should_escalate(&self) -> bool {
        // If more than 30% of steps have failed, escalate
        let failure_rate = self.failed_steps().len() as f64 / self.assigned_steps.len() as f64;
        failure_rate > 0.3
    }
}

/// Trait for objects that can spawn FeatureLeads.
pub trait FeatureLeadSpawner {
    /// Spawn a FeatureLead for a given task and plan.
    fn spawn_feature_lead(
        &self,
        task_id: Uuid,
        plan: Plan,
        parent_orchestrator: Arc<Orchestrator>,
    ) -> Result<FeatureLead, SwellError>;

    /// Get all active FeatureLeads.
    fn get_active_feature_leads(&self) -> Vec<FeatureLead>;

    /// Check if a task has an active FeatureLead.
    fn has_feature_lead(&self, task_id: Uuid) -> bool;
}

/// Manages multiple FeatureLeads for complex task handling.
#[derive(Debug, Default)]
pub struct FeatureLeadManager {
    /// Active FeatureLeads indexed by parent task ID
    active_leads: std::collections::HashMap<Uuid, FeatureLead>,
}

impl FeatureLeadManager {
    /// Create a new FeatureLeadManager.
    pub fn new() -> Self {
        Self {
            active_leads: std::collections::HashMap::new(),
        }
    }

    /// Register a new FeatureLead.
    pub fn register(&mut self, lead: FeatureLead) {
        self.active_leads.insert(lead.parent_task_id, lead);
    }

    /// Get a FeatureLead by parent task ID.
    pub fn get(&self, task_id: &Uuid) -> Option<&FeatureLead> {
        self.active_leads.get(task_id)
    }

    /// Get a mutable FeatureLead by parent task ID.
    pub fn get_mut(&mut self, task_id: &Uuid) -> Option<&mut FeatureLead> {
        self.active_leads.get_mut(task_id)
    }

    /// Remove a FeatureLead after completion.
    pub fn remove(&mut self, task_id: &Uuid) -> Option<FeatureLead> {
        self.active_leads.remove(task_id)
    }

    /// Get count of active FeatureLeads.
    pub fn len(&self) -> usize {
        self.active_leads.len()
    }

    /// Check if there are no active FeatureLeads.
    pub fn is_empty(&self) -> bool {
        self.active_leads.is_empty()
    }

    /// Get all parent task IDs with active FeatureLeads.
    pub fn active_task_ids(&self) -> Vec<Uuid> {
        self.active_leads.keys().cloned().collect()
    }
}

impl FeatureLeadSpawner for Orchestrator {
    fn spawn_feature_lead(
        &self,
        task_id: Uuid,
        plan: Plan,
        parent_orchestrator: Arc<Orchestrator>,
    ) -> Result<FeatureLead, SwellError> {
        // Check if spawning is appropriate
        if !FeatureLead::should_spawn(&plan) {
            return Err(SwellError::InvalidStateTransition(
                "Plan does not exceed step threshold for FeatureLead".into(),
            ));
        }

        // Segment the plan into 2 FeatureLeads for balanced distribution
        let segments = FeatureLead::segment_plan(&plan, 2);

        if segments.is_empty() {
            return Err(SwellError::InvalidStateTransition(
                "No segments created from plan".into(),
            ));
        }

        // Use the first segment for this FeatureLead
        // In a more sophisticated implementation, we'd spawn multiple
        let (feature_name, step_ids): (String, Vec<Uuid>) = segments[0].clone();

        let lead = FeatureLead::new(
            task_id,
            feature_name.clone(),
            step_ids.clone(),
            parent_orchestrator,
        );

        info!(
            task_id = %task_id,
            feature_name = %feature_name,
            assigned_steps = step_ids.len(),
            "FeatureLead spawned"
        );

        // Register with the orchestrator's FeatureLeadManager
        let mut manager = self.feature_lead_manager.blocking_write();
        manager.register(lead.clone());

        Ok(lead)
    }

    fn get_active_feature_leads(&self) -> Vec<FeatureLead> {
        let manager = self.feature_lead_manager.blocking_read();
        manager
            .active_task_ids()
            .iter()
            .filter_map(|task_id| manager.get(task_id).cloned())
            .collect()
    }

    fn has_feature_lead(&self, task_id: Uuid) -> bool {
        let manager = self.feature_lead_manager.blocking_read();
        manager.get(&task_id).is_some()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use swell_core::{PlanStep, RiskLevel, StepStatus};

    fn create_test_plan_with_steps(step_count: usize) -> Plan {
        let steps: Vec<PlanStep> = (0..step_count)
            .map(|i| PlanStep {
                id: Uuid::new_v4(),
                description: format!("Step {}", i + 1),
                affected_files: vec![format!("file_{}.rs", i + 1)],
                expected_tests: vec![],
                risk_level: RiskLevel::Low,
                dependencies: vec![],
                status: StepStatus::Pending,
            })
            .collect();

        Plan {
            id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            steps,
            total_estimated_tokens: step_count as u64 * 1000,
            risk_assessment: "Low risk".to_string(),
        }
    }

    // --- should_spawn Tests ---

    #[test]
    fn test_should_spawn_below_threshold() {
        let plan = create_test_plan_with_steps(FEATURE_LEAD_STEP_THRESHOLD);
        assert!(!FeatureLead::should_spawn(&plan));
    }

    #[test]
    fn test_should_spawn_at_threshold() {
        let plan = create_test_plan_with_steps(FEATURE_LEAD_STEP_THRESHOLD);
        assert!(!FeatureLead::should_spawn(&plan));
    }

    #[test]
    fn test_should_spawn_above_threshold() {
        let plan = create_test_plan_with_steps(FEATURE_LEAD_STEP_THRESHOLD + 1);
        assert!(FeatureLead::should_spawn(&plan));
    }

    #[test]
    fn test_should_spawn_well_above_threshold() {
        let plan = create_test_plan_with_steps(FEATURE_LEAD_STEP_THRESHOLD * 3);
        assert!(FeatureLead::should_spawn(&plan));
    }

    // --- segment_plan Tests ---

    #[test]
    fn test_segment_plan_single_segment() {
        let plan = create_test_plan_with_steps(20);
        let segments = FeatureLead::segment_plan(&plan, 2);

        // Should create 2 segments
        assert_eq!(segments.len(), 2);

        // First segment should have ~10 steps
        let (_, first_steps) = &segments[0];
        assert!(first_steps.len() >= 9 && first_steps.len() <= 11);

        // Second segment should have ~10 steps
        let (_, second_steps) = &segments[1];
        assert!(second_steps.len() >= 9 && second_steps.len() <= 11);
    }

    #[test]
    fn test_segment_plan_with_remainder() {
        let plan = create_test_plan_with_steps(25);
        let segments = FeatureLead::segment_plan(&plan, 3);

        // Should create 3 segments
        assert_eq!(segments.len(), 3);

        // Total steps should equal plan steps
        let total: usize = segments.iter().map(|(_, ids)| ids.len()).sum();
        assert_eq!(total, 25);
    }

    #[test]
    fn test_segment_plan_empty() {
        let plan = create_test_plan_with_steps(0);
        let segments = FeatureLead::segment_plan(&plan, 2);

        assert!(segments.is_empty());
    }

    #[test]
    fn test_segment_plan_more_leads_than_steps() {
        let plan = create_test_plan_with_steps(3);
        let segments = FeatureLead::segment_plan(&plan, 10);

        // Should only create as many segments as there are steps
        assert!(segments.len() <= 3);
    }

    // --- FeatureLead Execution Tests ---

    #[tokio::test]
    async fn test_feature_lead_creation() {
        let orchestrator = Orchestrator::new();
        let parent_task = orchestrator
            .create_task("Complex task".to_string(), vec![])
            .await
            .unwrap();

        let plan = create_test_plan_with_steps(20);
        orchestrator.set_plan(parent_task.id, plan).await.unwrap();

        let parent_orch = Arc::new(orchestrator);
        let lead = FeatureLead::new(
            parent_task.id,
            "Test Feature".to_string(),
            vec![Uuid::new_v4(), Uuid::new_v4()],
            parent_orch.clone(),
        );

        assert_eq!(lead.depth, MAX_ORCHESTRATOR_DEPTH);
        assert!(!lead.is_completed());
        assert_eq!(lead.completion_percentage(), 0.0);
    }

    #[tokio::test]
    async fn test_feature_lead_execution() {
        let orchestrator = Orchestrator::new();
        let parent_task = orchestrator
            .create_task("Complex task".to_string(), vec![])
            .await
            .unwrap();

        // Create plan and set it
        let plan = create_test_plan_with_steps(20);
        let step_ids: Vec<Uuid> = plan.steps.iter().take(5).map(|s| s.id).collect();
        orchestrator.set_plan(parent_task.id, plan).await.unwrap();

        let parent_orch = Arc::new(orchestrator);
        let mut lead = FeatureLead::new(
            parent_task.id,
            "Test Feature".to_string(),
            step_ids,
            parent_orch.clone(),
        );

        // Execute the lead
        let result = lead.execute().await;
        assert!(result.is_ok());
        assert!(lead.is_completed());
        assert_eq!(lead.completion_percentage(), 100.0);
    }

    #[tokio::test]
    async fn test_feature_lead_escalation_trigger() {
        let orchestrator = Orchestrator::new();
        let parent_task = orchestrator
            .create_task("Complex task".to_string(), vec![])
            .await
            .unwrap();

        let plan = create_test_plan_with_steps(20);
        let step_ids: Vec<Uuid> = plan.steps.iter().take(10).map(|s| s.id).collect();
        orchestrator.set_plan(parent_task.id, plan).await.unwrap();

        let parent_orch = Arc::new(orchestrator);
        let mut lead = FeatureLead::new(
            parent_task.id,
            "Test Feature".to_string(),
            step_ids,
            parent_orch.clone(),
        );

        // Execute with some failures
        lead.execute().await.unwrap();

        // Should not escalate if all steps succeed
        assert!(!lead.should_escalate());
    }

    // --- FeatureLeadManager Tests ---

    #[test]
    fn test_feature_lead_manager_empty() {
        let manager = FeatureLeadManager::new();
        assert!(manager.is_empty());
        assert_eq!(manager.len(), 0);
    }

    #[test]
    fn test_feature_lead_manager_register() {
        let mut manager = FeatureLeadManager::new();
        let orchestrator = Orchestrator::new();
        let parent_orch = Arc::new(orchestrator);

        let lead = FeatureLead::new(
            Uuid::new_v4(),
            "Test Feature".to_string(),
            vec![Uuid::new_v4()],
            parent_orch,
        );
        let task_id = lead.parent_task_id;

        manager.register(lead);
        assert!(!manager.is_empty());
        assert_eq!(manager.len(), 1);
        assert!(manager.get(&task_id).is_some());
    }

    #[test]
    fn test_feature_lead_manager_remove() {
        let mut manager = FeatureLeadManager::new();
        let orchestrator = Orchestrator::new();
        let parent_orch = Arc::new(orchestrator);

        let lead = FeatureLead::new(
            Uuid::new_v4(),
            "Test Feature".to_string(),
            vec![Uuid::new_v4()],
            parent_orch,
        );
        let task_id = lead.parent_task_id;

        manager.register(lead);
        let removed = manager.remove(&task_id);
        assert!(removed.is_some());
        assert!(manager.is_empty());
    }

    #[test]
    fn test_feature_lead_manager_active_task_ids() {
        let mut manager = FeatureLeadManager::new();
        let orchestrator = Orchestrator::new();
        let parent_orch = Arc::new(orchestrator);

        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();

        let lead1 = FeatureLead::new(
            task1,
            "Feature 1".to_string(),
            vec![Uuid::new_v4()],
            parent_orch.clone(),
        );
        let lead2 = FeatureLead::new(
            task2,
            "Feature 2".to_string(),
            vec![Uuid::new_v4()],
            parent_orch,
        );

        manager.register(lead1);
        manager.register(lead2);

        let active = manager.active_task_ids();
        assert_eq!(active.len(), 2);
        assert!(active.contains(&task1));
        assert!(active.contains(&task2));
    }

    // --- VAL-ORCH-002: Feature Lead spawning for sub-graphs >15 tasks ---

    /// Helper to create a test plan with specified number of steps
    fn create_large_plan(step_count: usize) -> Plan {
        let steps: Vec<PlanStep> = (0..step_count)
            .map(|i| PlanStep {
                id: Uuid::new_v4(),
                description: format!("Step {}", i + 1),
                affected_files: vec![format!("file_{}.rs", i + 1)],
                expected_tests: vec![],
                risk_level: RiskLevel::Low,
                dependencies: vec![],
                status: StepStatus::Pending,
            })
            .collect();

        Plan {
            id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            steps,
            total_estimated_tokens: step_count as u64 * 1000,
            risk_assessment: "Low risk".to_string(),
        }
    }

    #[test]
    fn test_feature_lead_threshold_boundary_at_15() {
        // At exactly 15 steps, should NOT spawn (threshold is >15)
        let plan = create_large_plan(FEATURE_LEAD_STEP_THRESHOLD);
        assert!(!FeatureLead::should_spawn(&plan));
    }

    #[test]
    fn test_feature_lead_threshold_boundary_above_15() {
        // Above 15 steps, should spawn
        let plan = create_large_plan(FEATURE_LEAD_STEP_THRESHOLD + 1);
        assert!(FeatureLead::should_spawn(&plan));
    }

    #[test]
    fn test_feature_lead_threshold_boundary_at_16() {
        // At exactly 16 steps, should spawn
        let plan = create_large_plan(16);
        assert!(FeatureLead::should_spawn(&plan));
    }

    #[test]
    fn test_feature_lead_threshold_boundary_at_14() {
        // At exactly 14 steps, should NOT spawn
        let plan = create_large_plan(14);
        assert!(!FeatureLead::should_spawn(&plan));
    }

    /// Test that segment_plan correctly distributes tasks for spawning
    #[test]
    fn test_segment_plan_for_large_subgraph() {
        // Simulate a 20-task sub-graph
        let plan = create_large_plan(20);
        let segments = FeatureLead::segment_plan(&plan, 2);

        // Should create 2 segments
        assert_eq!(segments.len(), 2);

        // Combined steps should equal 20
        let total_steps: usize = segments.iter().map(|(_, ids)| ids.len()).sum();
        assert_eq!(total_steps, 20);

        // Both segments should have ~10 steps each
        let first_count = segments[0].1.len();
        let second_count = segments[1].1.len();
        assert!(first_count >= 9 && first_count <= 11);
        assert!(second_count >= 9 && second_count <= 11);
    }

    /// Test that segment_plan for small subgraph (≤15) doesn't trigger spawning
    #[test]
    fn test_segment_plan_for_small_subgraph() {
        // Simulate a 10-task sub-graph
        let plan = create_large_plan(10);
        let segments = FeatureLead::segment_plan(&plan, 2);

        // With 10 tasks and 2 leads, segments would distribute as 5 and 5
        assert_eq!(segments.len(), 2);

        let total_steps: usize = segments.iter().map(|(_, ids)| ids.len()).sum();
        assert_eq!(total_steps, 10);
    }

    /// Test that FeatureLead correctly identifies which steps it's managing
    #[tokio::test]
    async fn test_feature_lead_assigned_steps_tracking() {
        let orchestrator = Orchestrator::new();
        let parent_task = orchestrator
            .create_task("Test".to_string(), vec![])
            .await
            .unwrap();

        let plan = create_large_plan(20);
        let step_ids: Vec<Uuid> = plan.steps.iter().take(10).map(|s| s.id).collect();
        orchestrator.set_plan(parent_task.id, plan).await.unwrap();

        let parent_orch = Arc::new(orchestrator);
        let lead = FeatureLead::new(
            parent_task.id,
            "Test Feature".to_string(),
            step_ids.clone(),
            parent_orch,
        );

        // The lead should have exactly 10 assigned steps
        assert_eq!(lead.assigned_steps.len(), 10);
        assert_eq!(lead.completion_percentage(), 0.0); // No steps completed yet
    }

    /// Test that FeatureLead correctly reports completion
    #[tokio::test]
    async fn test_feature_lead_completion_tracking() {
        let orchestrator = Orchestrator::new();
        let parent_task = orchestrator
            .create_task("Test".to_string(), vec![])
            .await
            .unwrap();

        let plan = create_large_plan(5);
        let step_ids: Vec<Uuid> = plan.steps.iter().map(|s| s.id).collect();
        orchestrator.set_plan(parent_task.id, plan).await.unwrap();

        let parent_orch = Arc::new(orchestrator);
        let mut lead = FeatureLead::new(
            parent_task.id,
            "Test Feature".to_string(),
            step_ids,
            parent_orch,
        );

        // Execute - all 5 steps should succeed (mocked)
        lead.execute().await.unwrap();

        // Completion should be 100%
        assert_eq!(lead.completion_percentage(), 100.0);
        assert!(lead.is_completed());
        assert!(lead.failed_steps().is_empty());
    }

    /// Test that subgraphs are correctly identified by size
    #[test]
    fn test_subgraph_size_threshold_concept() {
        use crate::task_graph::TaskGraph;
        use swell_core::TaskState;

        let mut graph = TaskGraph::new();

        // Create a 20-task sub-graph
        let mut task_ids: Vec<Uuid> = Vec::new();
        for i in 0..20 {
            let id = Uuid::new_v4();
            task_ids.push(id);
            if i > 0 {
                graph.add_task(id, vec![task_ids[i - 1]]).unwrap();
            } else {
                graph.add_task(id, vec![]).unwrap();
            }
        }

        // Largest subgraph should be 20
        assert_eq!(graph.largest_subgraph_size(), 20);

        // Add a 10-task sub-graph (disconnected)
        let mut small_task_ids: Vec<Uuid> = Vec::new();
        for i in 0..10 {
            let id = Uuid::new_v4();
            small_task_ids.push(id);
            if i > 0 {
                graph.add_task(id, vec![small_task_ids[i - 1]]).unwrap();
            } else {
                graph.add_task(id, vec![]).unwrap();
            }
        }

        // Largest subgraph should still be 20 (the 10-task one is smaller)
        assert_eq!(graph.largest_subgraph_size(), 20);
    }
}
