//! Agent pool and agent implementations.

use swell_core::traits::Agent;
use swell_core::{AgentRole, AgentId, SwellError, AgentContext, AgentResult};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;
use tracing::{info, debug};

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
}

impl PlannerAgent {
    pub fn new(model: String) -> Self {
        Self {
            model,
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

#[async_trait]
impl Agent for PlannerAgent {
    fn role(&self) -> AgentRole {
        AgentRole::Planner
    }

    fn description(&self) -> String {
        "Creates execution plans from task descriptions".to_string()
    }

    async fn execute(&self, context: AgentContext) -> Result<AgentResult, SwellError> {
        // For MVP, generate a simple plan
        // Full implementation would call the LLM
        
        let plan_json = serde_json::json!({
            "steps": [
                {
                    "id": Uuid::new_v4().to_string(),
                    "description": format!("Implement: {}", context.task.description),
                    "affected_files": [],
                    "expected_tests": [],
                    "risk_level": "medium",
                    "dependencies": [],
                    "status": "pending"
                }
            ],
            "total_estimated_tokens": 5000,
            "risk_assessment": "Standard implementation task"
        });

        Ok(AgentResult {
            success: true,
            output: serde_json::to_string(&plan_json).unwrap_or_default(),
            tool_calls: vec![],
            tokens_used: 500,
            error: None,
        })
    }
}

/// Generator agent - implements code based on plans
pub struct GeneratorAgent {
    model: String,
}

impl GeneratorAgent {
    pub fn new(model: String) -> Self {
        Self { model }
    }
}

#[async_trait]
impl Agent for GeneratorAgent {
    fn role(&self) -> AgentRole {
        AgentRole::Generator
    }

    fn description(&self) -> String {
        "Generates code implementations from plans".to_string()
    }

    async fn execute(&self, context: AgentContext) -> Result<AgentResult, SwellError> {
        // For MVP, simulate generation
        // Full implementation would use tools to edit files
        
        Ok(AgentResult {
            success: true,
            output: format!("Generated code for: {}", context.task.description),
            tool_calls: vec![],
            tokens_used: 1000,
            error: None,
        })
    }
}

/// Evaluator agent - validates code quality
pub struct EvaluatorAgent {
    model: String,
}

impl EvaluatorAgent {
    pub fn new(model: String) -> Self {
        Self { model }
    }
}

#[async_trait]
impl Agent for EvaluatorAgent {
    fn role(&self) -> AgentRole {
        AgentRole::Evaluator
    }

    fn description(&self) -> String {
        "Evaluates code quality and correctness".to_string()
    }

    async fn execute(&self, context: AgentContext) -> Result<AgentResult, SwellError> {
        // For MVP, simulate evaluation
        // Full implementation would run validation gates
        
        Ok(AgentResult {
            success: true,
            output: "Evaluation passed".to_string(),
            tool_calls: vec![],
            tokens_used: 300,
            error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swell_core::{Task, MemoryBlock, MemoryBlockType, LlmMessage, LlmRole, LlmConfig};

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

    #[tokio::test]
    async fn test_planner_agent() {
        let agent = PlannerAgent::new("claude-sonnet".to_string());
        
        let task = Task::new("Add user authentication".to_string());
        let context = AgentContext {
            task,
            memory_blocks: vec![],
            session_id: Uuid::new_v4(),
            workspace_path: None,
        };
        
        let result = agent.execute(context).await.unwrap();
        assert!(result.success);
        
        // Parse the plan from output
        let plan: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert!(plan["steps"].is_array());
    }

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
    }
}
