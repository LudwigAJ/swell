//! Orchestrator crate - coordinates multi-agent task execution.
//!
//! # Architecture
//!
//! The orchestrator manages:
//! - [`Orchestrator`] - main coordinator
//! - [`TaskStateMachine`] - state transitions
//! - [`AgentPool`] - manages agent instances
//! - [`ExecutionController`] - handles parallel execution

pub mod state_machine;
pub mod agents;
pub mod execution;

pub use state_machine::TaskStateMachine;
pub use agents::{AgentPool, AgentHandle, PlannerAgent, GeneratorAgent, EvaluatorAgent};
pub use execution::ExecutionController;

use swell_core::{
    Task, TaskState, AgentRole, Plan, AgentId, SwellError,
    ValidationResult, AgentContext, AgentResult,
};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{info, error, warn, debug};
use uuid::Uuid;

/// Maximum concurrent agents
pub const MAX_CONCURRENT_AGENTS: usize = 6;

/// Events emitted by the orchestrator
#[derive(Debug, Clone)]
pub enum OrchestratorEvent {
    TaskCreated(Uuid),
    TaskStateChanged { task_id: Uuid, from: TaskState, to: TaskState },
    AgentStarted { agent_id: AgentId, task_id: Uuid },
    AgentFinished { agent_id: AgentId, task_id: Uuid },
    ExecutionProgress { task_id: Uuid, message: String },
}

/// The main orchestrator that coordinates agents and tasks
pub struct Orchestrator {
    state_machine: Arc<RwLock<TaskStateMachine>>,
    agent_pool: Arc<RwLock<AgentPool>>,
    event_sender: mpsc::UnboundedSender<OrchestratorEvent>,
}

impl Orchestrator {
    /// Create a new orchestrator
    pub fn new() -> Self {
        let (tx, _rx) = mpsc::unbounded_channel();
        
        Self {
            state_machine: Arc::new(RwLock::new(TaskStateMachine::new())),
            agent_pool: Arc::new(RwLock::new(AgentPool::new())),
            event_sender: tx,
        }
    }

    /// Create a new task
    pub async fn create_task(&self, description: String) -> Task {
        let task = {
            let mut sm = self.state_machine.write().await;
            sm.create_task(description)
        };
        let _ = self.event_sender.send(OrchestratorEvent::TaskCreated(task.id));
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

        let _ = self.event_sender.send(OrchestratorEvent::AgentStarted { agent_id, task_id });
        Ok(agent_id)
    }

    /// Release an agent back to the pool
    pub async fn release_agent(&self, agent_id: AgentId, task_id: Uuid) {
        let _ = {
            let mut pool = self.agent_pool.write().await;
            pool.release(agent_id)
        };
        let _ = self.event_sender.send(OrchestratorEvent::AgentFinished { agent_id, task_id });
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
        
        sm.enrich_task(task_id)?;
        
        let task = sm.get_task(task_id)?;
        if task.plan.is_none() {
            return Err(SwellError::InvalidStateTransition("Cannot start task without plan".into()));
        }
        
        sm.ready_task(task_id)?;
        sm.assign_task(task_id, Uuid::nil())?; // Will be reassigned when agent picks it up
        sm.start_execution(task_id)?;
        
        Ok(())
    }

    /// Transition to validating state
    pub async fn start_validation(&self, task_id: Uuid) -> Result<(), SwellError> {
        let mut sm = self.state_machine.write().await;
        sm.start_validation(task_id)
    }

    /// Complete task with validation result
    pub async fn complete_task(&self, task_id: Uuid, result: ValidationResult) -> Result<(), SwellError> {
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
            
            // Check for escalation
            if let Ok(task) = sm.get_task(task_id) {
                if task.iteration_count >= 3 {
                    sm.escalate_task(task_id)?;
                    warn!(task_id = %task_id, "Task escalated after 3 failures");
                }
            }
        }
        
        Ok(())
    }

    /// Get the state machine for direct access (use sparingly)
    pub fn state_machine(&self) -> Arc<RwLock<TaskStateMachine>> {
        self.state_machine.clone()
    }
}

impl Default for Orchestrator {
    fn default() -> Self {
        Self::new()
    }
}
