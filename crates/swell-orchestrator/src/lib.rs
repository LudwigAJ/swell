pub mod state_machine;

pub use state_machine::TaskStateMachine;

use swell_core::{Task, TaskState, Agent, AgentRole, Plan, AgentId, SwellError};
use tracing::{info, error, debug};

/// The main orchestrator that coordinates agents and tasks
pub struct Orchestrator {
    state_machine: TaskStateMachine,
    agents: std::collections::HashMap<AgentId, Agent>,
}

impl Orchestrator {
    pub fn new() -> Self {
        info!("Initializing orchestrator");
        Self {
            state_machine: TaskStateMachine::new(),
            agents: std::collections::HashMap::new(),
        }
    }

    /// Create a new task from a description
    pub fn create_task(&mut self, description: String) -> Task {
        self.state_machine.create_task(description)
    }

    /// Get task by ID
    pub fn get_task(&self, id: uuid::Uuid) -> Result<Task, SwellError> {
        self.state_machine.get_task(id)
    }

    /// Register an agent
    pub fn register_agent(&mut self, role: AgentRole, model: String) -> Agent {
        let agent = Agent::new(role, model);
        info!(agent_id = %agent.id, role = ?agent.role, "Registered agent");
        self.agents.insert(agent.id, agent.clone());
        agent
    }

    /// Get agent by ID
    pub fn get_agent(&self, id: AgentId) -> Option<&Agent> {
        self.agents.get(&id)
    }

    /// Create a plan for a task
    pub fn set_task_plan(&mut self, task_id: uuid::Uuid, plan: Plan) -> Result<(), SwellError> {
        self.state_machine.set_plan(task_id, plan)
    }

    /// Advance task to READY (after planning)
    pub fn approve_plan(&mut self, task_id: uuid::Uuid) -> Result<(), SwellError> {
        // First enrich, then set ready
        self.state_machine.enrich_task(task_id)?;
        self.state_machine.ready_task(task_id)
    }

    /// Assign task to an available agent of the right role
    pub fn assign_task(&mut self, task_id: uuid::Uuid) -> Result<Agent, SwellError> {
        // Find a free agent of appropriate role
        // For MVP, we use Generator role for all execution
        let agent = self.agents.values()
            .find(|a| a.role == AgentRole::Generator && a.current_task.is_none())
            .cloned()
            .ok_or_else(|| SwellError::AgentNotFound(uuid::Uuid::nil()))?;

        self.state_machine.assign_task(task_id, agent.id)?;
        Ok(agent)
    }

    /// Start executing a ready task
    pub fn start_execution(&mut self, task_id: uuid::Uuid) -> Result<(), SwellError> {
        self.state_machine.start_execution(task_id)
    }

    /// Start validation for a task
    pub fn start_validation(&mut self, task_id: uuid::Uuid) -> Result<(), SwellError> {
        self.state_machine.start_validation(task_id)
    }

    /// Finalize task after validation
    pub fn finalize_task(&mut self, task_id: uuid::Uuid, passed: bool) -> Result<(), SwellError> {
        if passed {
            self.state_machine.accept_task(task_id)?;
            info!(task_id = %task_id, "Task accepted");
        } else {
            self.state_machine.reject_task(task_id)?;
            info!(task_id = %task_id, "Task rejected");

            // Check if we should escalate
            let task = self.state_machine.get_task(task_id)?;
            if task.iteration_count >= 3 {
                self.state_machine.escalate_task(task_id)?;
                error!(task_id = %task_id, "Task escalated after 3 failures");
            }
        }
        Ok(())
    }

    /// Get all tasks
    pub fn get_all_tasks(&self) -> Vec<Task> {
        self.state_machine.get_all_tasks()
    }

    /// Get tasks by state
    pub fn get_tasks_by_state(&self, state: TaskState) -> Vec<Task> {
        self.state_machine.get_tasks_by_state(state)
    }

    /// Check if any task has exceeded budget and should be killed
    pub fn check_safety_limits(&self, task_id: uuid::Uuid) -> Result<(), SwellError> {
        let task = self.state_machine.get_task(task_id)?;
        if task.tokens_used >= task.token_budget {
            return Err(SwellError::BudgetExceeded(format!(
                "Task {} exceeded token budget", task_id
            )));
        }
        if task.iteration_count >= 10 {
            return Err(SwellError::DoomLoopDetected);
        }
        Ok(())
    }
}

impl Default for Orchestrator {
    fn default() -> Self {
        Self::new()
    }
}
