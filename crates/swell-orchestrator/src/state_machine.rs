use std::collections::HashMap;
use swell_core::{Plan, SwellError, Task, TaskState};
use tracing::{info, warn};

/// Task state machine implementing the 8-state lifecycle from the spec
pub struct TaskStateMachine {
    tasks: HashMap<uuid::Uuid, Task>,
}

impl TaskStateMachine {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
        }
    }

    pub fn create_task(&mut self, description: String) -> Task {
        let task = Task::new(description);
        let id = task.id;
        info!(task_id = %id, "Creating new task");
        self.tasks.insert(id, task);
        self.tasks.get(&id).unwrap().clone()
    }

    pub fn get_task(&self, id: uuid::Uuid) -> Result<Task, SwellError> {
        self.tasks
            .get(&id)
            .cloned()
            .ok_or(SwellError::TaskNotFound(id))
    }

    pub fn get_task_mut(&mut self, id: uuid::Uuid) -> Result<&mut Task, SwellError> {
        self.tasks.get_mut(&id).ok_or(SwellError::TaskNotFound(id))
    }

    /// Transition task to ENRICHED state
    pub fn enrich_task(&mut self, id: uuid::Uuid) -> Result<(), SwellError> {
        let task = self.get_task_mut(id)?;
        match task.state {
            TaskState::Created => {
                task.transition_to(TaskState::Enriched);
                Ok(())
            }
            _ => Err(SwellError::InvalidStateTransition(format!(
                "Cannot enrich task in state {}",
                task.state
            ))),
        }
    }

    /// Transition task to READY state (plan approved)
    pub fn ready_task(&mut self, id: uuid::Uuid) -> Result<(), SwellError> {
        let task = self.get_task_mut(id)?;
        match task.state {
            TaskState::Enriched => {
                if task.plan.is_none() {
                    return Err(SwellError::InvalidStateTransition(
                        "Cannot ready task without a plan".to_string(),
                    ));
                }
                task.transition_to(TaskState::Ready);
                Ok(())
            }
            _ => Err(SwellError::InvalidStateTransition(format!(
                "Cannot ready task in state {}",
                task.state
            ))),
        }
    }

    /// Assign task to an agent
    pub fn assign_task(&mut self, id: uuid::Uuid, agent_id: uuid::Uuid) -> Result<(), SwellError> {
        let task = self.get_task_mut(id)?;
        match task.state {
            TaskState::Ready => {
                task.assigned_agent = Some(agent_id);
                task.transition_to(TaskState::Assigned);
                Ok(())
            }
            _ => Err(SwellError::InvalidStateTransition(format!(
                "Cannot assign task in state {}",
                task.state
            ))),
        }
    }

    /// Start executing the task
    pub fn start_execution(&mut self, id: uuid::Uuid) -> Result<(), SwellError> {
        let task = self.get_task_mut(id)?;
        match task.state {
            TaskState::Assigned => {
                task.transition_to(TaskState::Executing);
                Ok(())
            }
            _ => Err(SwellError::InvalidStateTransition(format!(
                "Cannot start executing task in state {}",
                task.state
            ))),
        }
    }

    /// Start validation phase
    pub fn start_validation(&mut self, id: uuid::Uuid) -> Result<(), SwellError> {
        let task = self.get_task_mut(id)?;
        match task.state {
            TaskState::Executing => {
                task.transition_to(TaskState::Validating);
                Ok(())
            }
            _ => Err(SwellError::InvalidStateTransition(format!(
                "Cannot validate task in state {}",
                task.state
            ))),
        }
    }

    /// Mark task as accepted (validation passed)
    pub fn accept_task(&mut self, id: uuid::Uuid) -> Result<(), SwellError> {
        let task = self.get_task_mut(id)?;
        match task.state {
            TaskState::Validating => {
                task.transition_to(TaskState::Accepted);
                Ok(())
            }
            _ => Err(SwellError::InvalidStateTransition(format!(
                "Cannot accept task in state {}",
                task.state
            ))),
        }
    }

    /// Mark task as rejected (validation failed)
    pub fn reject_task(&mut self, id: uuid::Uuid) -> Result<(), SwellError> {
        let task = self.get_task_mut(id)?;
        match task.state {
            TaskState::Validating => {
                task.transition_to(TaskState::Rejected);
                task.iteration_count += 1;
                Ok(())
            }
            _ => Err(SwellError::InvalidStateTransition(format!(
                "Cannot reject task in state {}",
                task.state
            ))),
        }
    }

    /// Transition from Rejected back to Ready for retry (orchestrator manages this)
    pub fn retry_task(&mut self, id: uuid::Uuid) -> Result<(), SwellError> {
        let task = self.get_task_mut(id)?;
        match task.state {
            TaskState::Rejected => {
                task.transition_to(TaskState::Ready);
                task.assigned_agent = None;
                Ok(())
            }
            _ => Err(SwellError::InvalidStateTransition(format!(
                "Cannot retry task in state {}",
                task.state
            ))),
        }
    }

    /// Mark task as failed (unrecoverable)
    pub fn fail_task(&mut self, id: uuid::Uuid) -> Result<(), SwellError> {
        let task = self.get_task_mut(id)?;
        task.transition_to(TaskState::Failed);
        Ok(())
    }

    /// Pause a task (operator intervention)
    pub fn pause_task(&mut self, id: uuid::Uuid, reason: String) -> Result<(), SwellError> {
        let task = self.get_task_mut(id)?;
        match task.state {
            TaskState::Executing | TaskState::Validating => {
                task.paused_reason = Some(reason);
                task.transition_to(TaskState::Paused);
                Ok(())
            }
            _ => Err(SwellError::InvalidStateTransition(format!(
                "Cannot pause task in state {}",
                task.state
            ))),
        }
    }

    /// Resume a paused task
    pub fn resume_task(&mut self, id: uuid::Uuid) -> Result<(), SwellError> {
        let task = self.get_task_mut(id)?;
        match task.state {
            TaskState::Paused => {
                task.paused_reason = None;
                // Resume to previous state - if was validating, go back to validating
                // otherwise go back to executing
                let previous_validating = task.validation_result.is_some();
                if previous_validating {
                    task.transition_to(TaskState::Validating);
                } else {
                    task.transition_to(TaskState::Executing);
                }
                Ok(())
            }
            _ => Err(SwellError::InvalidStateTransition(format!(
                "Cannot resume task in state {}",
                task.state
            ))),
        }
    }

    /// Inject instructions into a task
    pub fn inject_instruction(&mut self, id: uuid::Uuid, instruction: String) -> Result<(), SwellError> {
        let task = self.get_task_mut(id)?;
        // Can inject into any active state
        match task.state {
            TaskState::Created
            | TaskState::Enriched
            | TaskState::Ready
            | TaskState::Assigned
            | TaskState::Executing
            | TaskState::Paused
            | TaskState::Validating => {
                task.injected_instructions.push(instruction);
                tracing::info!(task_id = %id, instruction_count = task.injected_instructions.len(), "Instruction injected");
                Ok(())
            }
            _ => Err(SwellError::InvalidStateTransition(format!(
                "Cannot inject instructions into task in state {}",
                task.state
            ))),
        }
    }

    /// Modify task scope boundaries
    pub fn modify_scope(&mut self, id: uuid::Uuid, new_scope: swell_core::TaskScope) -> Result<(), SwellError> {
        let task = self.get_task_mut(id)?;
        // Store original scope if not already stored
        if task.original_scope.is_none() {
            task.original_scope = Some(task.current_scope.clone());
        }
        task.current_scope = new_scope;
        tracing::info!(task_id = %id, "Task scope modified");
        Ok(())
    }

    /// Restore original scope (revert modify_scope)
    pub fn restore_original_scope(&mut self, id: uuid::Uuid) -> Result<(), SwellError> {
        let task = self.get_task_mut(id)?;
        if let Some(original) = task.original_scope.take() {
            task.current_scope = original;
            tracing::info!(task_id = %id, "Task scope restored to original");
            Ok(())
        } else {
            Err(SwellError::InvalidStateTransition(
                "No original scope to restore".to_string(),
            ))
        }
    }

    /// Escalate task to human
    pub fn escalate_task(&mut self, id: uuid::Uuid) -> Result<(), SwellError> {
        let task = self.get_task_mut(id)?;
        task.transition_to(TaskState::Escalated);
        warn!(task_id = %id, "Task escalated to human");
        Ok(())
    }

    /// Set plan for task
    pub fn set_plan(&mut self, id: uuid::Uuid, plan: Plan) -> Result<(), SwellError> {
        let task = self.get_task_mut(id)?;
        task.plan = Some(plan);
        Ok(())
    }

    /// Check if task can proceed (dependencies satisfied)
    pub fn can_proceed(&self, id: uuid::Uuid) -> Result<bool, SwellError> {
        let task = self.get_task(id)?;
        for dep_id in &task.dependencies {
            let dep = self.get_task(*dep_id)?;
            if dep.state != TaskState::Accepted {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Get all tasks in a specific state
    pub fn get_tasks_by_state(&self, state: TaskState) -> Vec<Task> {
        self.tasks
            .values()
            .filter(|t| t.state == state)
            .cloned()
            .collect()
    }

    /// Get all tasks
    pub fn get_all_tasks(&self) -> Vec<Task> {
        self.tasks.values().cloned().collect()
    }
}

impl Default for TaskStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use swell_core::{Plan, PlanStep, RiskLevel, StepStatus};

    fn create_test_plan(task_id: uuid::Uuid) -> Plan {
        Plan {
            id: uuid::Uuid::new_v4(),
            task_id,
            steps: vec![PlanStep {
                id: uuid::Uuid::new_v4(),
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

    fn create_test_task_and_plan(sm: &mut TaskStateMachine) -> (uuid::Uuid, Plan) {
        let task = sm.create_task("Test task".to_string());
        let plan = create_test_plan(task.id);
        sm.set_plan(task.id, plan.clone()).unwrap();
        (task.id, plan)
    }

    // --- Valid Transition Tests ---

    #[test]
    fn test_created_to_enriched() {
        let mut sm = TaskStateMachine::new();
        let task = sm.create_task("Test".to_string());

        assert_eq!(task.state, TaskState::Created);

        sm.enrich_task(task.id).unwrap();

        let task = sm.get_task(task.id).unwrap();
        assert_eq!(task.state, TaskState::Enriched);
    }

    #[test]
    fn test_enriched_to_ready_with_plan() {
        let mut sm = TaskStateMachine::new();
        let (task_id, plan) = create_test_task_and_plan(&mut sm);

        sm.enrich_task(task_id).unwrap();
        sm.ready_task(task_id).unwrap();

        let task = sm.get_task(task_id).unwrap();
        assert_eq!(task.state, TaskState::Ready);
        assert!(task.plan.is_some());
        assert_eq!(task.plan.unwrap().id, plan.id);
    }

    #[test]
    fn test_ready_to_assigned() {
        let mut sm = TaskStateMachine::new();
        let (task_id, _) = create_test_task_and_plan(&mut sm);

        sm.enrich_task(task_id).unwrap();
        sm.ready_task(task_id).unwrap();

        let agent_id = uuid::Uuid::new_v4();
        sm.assign_task(task_id, agent_id).unwrap();

        let task = sm.get_task(task_id).unwrap();
        assert_eq!(task.state, TaskState::Assigned);
        assert_eq!(task.assigned_agent, Some(agent_id));
    }

    #[test]
    fn test_assigned_to_executing() {
        let mut sm = TaskStateMachine::new();
        let (task_id, _) = create_test_task_and_plan(&mut sm);

        sm.enrich_task(task_id).unwrap();
        sm.ready_task(task_id).unwrap();
        sm.assign_task(task_id, uuid::Uuid::new_v4()).unwrap();
        sm.start_execution(task_id).unwrap();

        let task = sm.get_task(task_id).unwrap();
        assert_eq!(task.state, TaskState::Executing);
    }

    #[test]
    fn test_executing_to_validating() {
        let mut sm = TaskStateMachine::new();
        let (task_id, _) = create_test_task_and_plan(&mut sm);

        sm.enrich_task(task_id).unwrap();
        sm.ready_task(task_id).unwrap();
        sm.assign_task(task_id, uuid::Uuid::new_v4()).unwrap();
        sm.start_execution(task_id).unwrap();
        sm.start_validation(task_id).unwrap();

        let task = sm.get_task(task_id).unwrap();
        assert_eq!(task.state, TaskState::Validating);
    }

    #[test]
    fn test_validating_to_accepted() {
        let mut sm = TaskStateMachine::new();
        let (task_id, _) = create_test_task_and_plan(&mut sm);

        sm.enrich_task(task_id).unwrap();
        sm.ready_task(task_id).unwrap();
        sm.assign_task(task_id, uuid::Uuid::new_v4()).unwrap();
        sm.start_execution(task_id).unwrap();
        sm.start_validation(task_id).unwrap();
        sm.accept_task(task_id).unwrap();

        let task = sm.get_task(task_id).unwrap();
        assert_eq!(task.state, TaskState::Accepted);
    }

    #[test]
    fn test_validating_to_rejected() {
        let mut sm = TaskStateMachine::new();
        let (task_id, _) = create_test_task_and_plan(&mut sm);

        sm.enrich_task(task_id).unwrap();
        sm.ready_task(task_id).unwrap();
        sm.assign_task(task_id, uuid::Uuid::new_v4()).unwrap();
        sm.start_execution(task_id).unwrap();
        sm.start_validation(task_id).unwrap();
        sm.reject_task(task_id).unwrap();

        let task = sm.get_task(task_id).unwrap();
        assert_eq!(task.state, TaskState::Rejected);
        assert_eq!(task.iteration_count, 1);
    }

    #[test]
    fn test_rejected_iteration_count_increments() {
        let mut sm = TaskStateMachine::new();
        let (task_id, _) = create_test_task_and_plan(&mut sm);

        sm.enrich_task(task_id).unwrap();
        sm.ready_task(task_id).unwrap();
        sm.assign_task(task_id, uuid::Uuid::new_v4()).unwrap();
        sm.start_execution(task_id).unwrap();
        sm.start_validation(task_id).unwrap();
        sm.reject_task(task_id).unwrap();

        let task = sm.get_task(task_id).unwrap();
        assert_eq!(task.state, TaskState::Rejected);
        assert_eq!(task.iteration_count, 1);

        // After rejection, task is in Rejected state
        // Further validation transitions are not allowed
        let result = sm.start_validation(task_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_escalation_after_3_failures() {
        let mut sm = TaskStateMachine::new();
        let task = sm.create_task("Test".to_string());
        let plan = create_test_plan(task.id);
        sm.set_plan(task.id, plan).unwrap();
        let task_id = task.id;

        // First cycle: Created → Enriched → Ready → Assigned → Executing → Validating → Rejected
        sm.enrich_task(task_id).unwrap();
        sm.ready_task(task_id).unwrap();
        sm.assign_task(task_id, uuid::Uuid::new_v4()).unwrap();
        sm.start_execution(task_id).unwrap();
        sm.start_validation(task_id).unwrap();
        sm.reject_task(task_id).unwrap();
        assert_eq!(sm.get_task(task_id).unwrap().iteration_count, 1);

        // Retry: Rejected → Ready → Assigned → Executing → Validating → Rejected
        sm.retry_task(task_id).unwrap();
        sm.assign_task(task_id, uuid::Uuid::new_v4()).unwrap();
        sm.start_execution(task_id).unwrap();
        sm.start_validation(task_id).unwrap();
        sm.reject_task(task_id).unwrap();
        assert_eq!(sm.get_task(task_id).unwrap().iteration_count, 2);

        // Second retry
        sm.retry_task(task_id).unwrap();
        sm.assign_task(task_id, uuid::Uuid::new_v4()).unwrap();
        sm.start_execution(task_id).unwrap();
        sm.start_validation(task_id).unwrap();
        sm.reject_task(task_id).unwrap();
        assert_eq!(sm.get_task(task_id).unwrap().iteration_count, 3);

        // After 3 rejections, escalate instead of retrying
        sm.escalate_task(task_id).unwrap();
        assert_eq!(sm.get_task(task_id).unwrap().state, TaskState::Escalated);
    }

    // --- Invalid Transition Tests ---

    #[test]
    fn test_cannot_enrich_non_created_task() {
        let mut sm = TaskStateMachine::new();
        let task_id = sm.create_task("Test".to_string()).id;

        sm.enrich_task(task_id).unwrap(); // Now in Enriched

        let result = sm.enrich_task(task_id);
        assert!(result.is_err());
        match result.unwrap_err() {
            SwellError::InvalidStateTransition(msg) => {
                assert!(msg.contains("Enriched") || msg.contains("state"));
            }
            _ => panic!("Expected InvalidStateTransition"),
        }
    }

    #[test]
    fn test_cannot_ready_task_without_plan() {
        let mut sm = TaskStateMachine::new();
        let task_id = sm.create_task("Test".to_string()).id;

        sm.enrich_task(task_id).unwrap();

        let result = sm.ready_task(task_id);
        assert!(result.is_err());
        match result.unwrap_err() {
            SwellError::InvalidStateTransition(msg) => {
                assert!(msg.contains("without a plan"));
            }
            _ => panic!("Expected InvalidStateTransition"),
        }
    }

    #[test]
    fn test_cannot_ready_non_enriched_task() {
        let mut sm = TaskStateMachine::new();
        let task_id = sm.create_task("Test".to_string()).id;

        // Skip enrich, go directly to ready
        let result = sm.ready_task(task_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_cannot_assign_non_ready_task() {
        let mut sm = TaskStateMachine::new();
        let (task_id, _) = create_test_task_and_plan(&mut sm);

        // Try to assign without going through Ready
        let result = sm.assign_task(task_id, uuid::Uuid::new_v4());
        assert!(result.is_err());
    }

    #[test]
    fn test_cannot_start_execution_non_assigned_task() {
        let mut sm = TaskStateMachine::new();
        let (task_id, _) = create_test_task_and_plan(&mut sm);

        sm.enrich_task(task_id).unwrap();
        sm.ready_task(task_id).unwrap();

        // Skip assign, try to start execution
        let result = sm.start_execution(task_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_cannot_start_validation_non_executing_task() {
        let mut sm = TaskStateMachine::new();
        let (task_id, _) = create_test_task_and_plan(&mut sm);

        sm.enrich_task(task_id).unwrap();
        sm.ready_task(task_id).unwrap();
        sm.assign_task(task_id, uuid::Uuid::new_v4()).unwrap();

        // Skip execution, try to start validation
        let result = sm.start_validation(task_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_cannot_accept_non_validating_task() {
        let mut sm = TaskStateMachine::new();
        let (task_id, _) = create_test_task_and_plan(&mut sm);

        sm.enrich_task(task_id).unwrap();
        sm.ready_task(task_id).unwrap();
        sm.assign_task(task_id, uuid::Uuid::new_v4()).unwrap();
        sm.start_execution(task_id).unwrap();

        // Try to accept without validating
        let result = sm.accept_task(task_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_cannot_reject_non_validating_task() {
        let mut sm = TaskStateMachine::new();
        let (task_id, _) = create_test_task_and_plan(&mut sm);

        sm.enrich_task(task_id).unwrap();
        sm.ready_task(task_id).unwrap();
        sm.assign_task(task_id, uuid::Uuid::new_v4()).unwrap();

        let result = sm.reject_task(task_id);
        assert!(result.is_err());
    }

    // --- Plan Attachment Tests ---

    #[test]
    fn test_plan_attached_after_set_plan() {
        let mut sm = TaskStateMachine::new();
        let task = sm.create_task("Test".to_string());

        assert!(sm.get_task(task.id).unwrap().plan.is_none());

        let plan = create_test_plan(task.id);
        sm.set_plan(task.id, plan.clone()).unwrap();

        let retrieved_task = sm.get_task(task.id).unwrap();
        assert!(retrieved_task.plan.is_some());
        assert_eq!(retrieved_task.plan.unwrap().id, plan.id);
    }

    #[test]
    fn test_plan_preserved_through_state_transitions() {
        let mut sm = TaskStateMachine::new();
        let (task_id, plan) = create_test_task_and_plan(&mut sm);

        sm.enrich_task(task_id).unwrap();
        sm.ready_task(task_id).unwrap();

        let task = sm.get_task(task_id).unwrap();
        assert!(task.plan.is_some());
        assert_eq!(task.plan.unwrap().id, plan.id);
    }

    // --- Helper Methods Tests ---

    #[test]
    fn test_get_tasks_by_state() {
        let mut sm = TaskStateMachine::new();

        let task1 = sm.create_task("Task 1".to_string());
        let task2 = sm.create_task("Task 2".to_string());

        sm.enrich_task(task1.id).unwrap();

        let created_tasks = sm.get_tasks_by_state(TaskState::Created);
        let enriched_tasks = sm.get_tasks_by_state(TaskState::Enriched);

        assert_eq!(created_tasks.len(), 1);
        assert_eq!(created_tasks[0].id, task2.id);
        assert_eq!(enriched_tasks.len(), 1);
        assert_eq!(enriched_tasks[0].id, task1.id);
    }

    #[test]
    fn test_get_all_tasks() {
        let mut sm = TaskStateMachine::new();

        let task1 = sm.create_task("Task 1".to_string());
        let task2 = sm.create_task("Task 2".to_string());

        let all = sm.get_all_tasks();
        assert_eq!(all.len(), 2);

        let ids: Vec<_> = all.iter().map(|t| t.id).collect();
        assert!(ids.contains(&task1.id));
        assert!(ids.contains(&task2.id));
    }

    #[test]
    fn test_task_not_found_error() {
        let sm = TaskStateMachine::new();
        let fake_id = uuid::Uuid::new_v4();

        let result = sm.get_task(fake_id);
        assert!(result.is_err());

        match result.unwrap_err() {
            SwellError::TaskNotFound(id) => assert_eq!(id, fake_id),
            _ => panic!("Expected TaskNotFound"),
        }
    }

    // --- Fail and Escalate Tests ---

    #[test]
    fn test_fail_task_transitions_to_failed() {
        let mut sm = TaskStateMachine::new();
        let task_id = sm.create_task("Test".to_string()).id;

        sm.fail_task(task_id).unwrap();

        let task = sm.get_task(task_id).unwrap();
        assert_eq!(task.state, TaskState::Failed);
    }

    #[test]
    fn test_escalate_task_transitions_to_escalated() {
        let mut sm = TaskStateMachine::new();
        let task_id = sm.create_task("Test".to_string()).id;

        sm.escalate_task(task_id).unwrap();

        let task = sm.get_task(task_id).unwrap();
        assert_eq!(task.state, TaskState::Escalated);
    }

    #[test]
    fn test_set_plan_only_sets_plan() {
        let mut sm = TaskStateMachine::new();
        let task = sm.create_task("Test".to_string());

        assert_eq!(sm.get_task(task.id).unwrap().state, TaskState::Created);

        let plan = create_test_plan(task.id);
        sm.set_plan(task.id, plan.clone()).unwrap();

        // State should still be Created
        assert_eq!(sm.get_task(task.id).unwrap().state, TaskState::Created);
        // But plan should be set
        assert!(sm.get_task(task.id).unwrap().plan.is_some());
    }
}
