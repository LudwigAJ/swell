use swell_core::{Task, TaskState, SwellError, Plan};
use std::collections::HashMap;
use tracing::{info, warn, error};

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
        self.tasks.get(&id)
            .cloned()
            .ok_or(SwellError::TaskNotFound(id))
    }

    pub fn get_task_mut(&mut self, id: uuid::Uuid) -> Result<&mut Task, SwellError> {
        self.tasks.get_mut(&id)
            .ok_or(SwellError::TaskNotFound(id))
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
                "Cannot enrich task in state {}", task.state
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
                        "Cannot ready task without a plan".to_string()
                    ));
                }
                task.transition_to(TaskState::Ready);
                Ok(())
            }
            _ => Err(SwellError::InvalidStateTransition(format!(
                "Cannot ready task in state {}", task.state
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
                "Cannot assign task in state {}", task.state
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
                "Cannot start executing task in state {}", task.state
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
                "Cannot validate task in state {}", task.state
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
                "Cannot accept task in state {}", task.state
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
                "Cannot reject task in state {}", task.state
            ))),
        }
    }

    /// Mark task as failed (unrecoverable)
    pub fn fail_task(&mut self, id: uuid::Uuid) -> Result<(), SwellError> {
        let task = self.get_task_mut(id)?;
        task.transition_to(TaskState::Failed);
        Ok(())
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
        self.tasks.values()
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
