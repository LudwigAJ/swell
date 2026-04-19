//! Task Board module for tracking task execution state, agent assignments,
//! time spent, and token costs.
//!
//! This module provides a persistent task board that:
//! - Tracks task state transitions
//! - Records agent assignments
//! - Measures time spent per task
//! - Accumulates token costs per task
//!
//! # Example
//!
//! ```rust,ignore
//! use swell_orchestrator::task_board::{TaskBoard, TaskBoardEntry};
//!
//! let board = TaskBoard::new();
//!
//! // Start tracking a task
//! board.start_task(task_id, TaskState::Executing);
//!
//! // Assign agent
//! board.assign_agent(task_id, agent_id);
//!
//! // Record token usage
//! board.record_cost(task_id, 1000);
//!
//! // Complete task
//! board.complete_task(task_id, TaskState::Accepted);
//! ```

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use swell_core::ids::{AgentId, TaskId};

/// Cost model for estimating token costs
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CostModel {
    /// Cost per 1M input tokens in USD
    pub input_cost_per_million: f64,
    /// Cost per 1M output tokens in USD
    pub output_cost_per_million: f64,
}

impl Default for CostModel {
    fn default() -> Self {
        // Default to Claude 3.5 Sonnet pricing (example)
        Self {
            input_cost_per_million: 3.0,
            output_cost_per_million: 15.0,
        }
    }
}

impl CostModel {
    /// Calculate cost in USD for given token counts
    pub fn calculate_cost(&self, input_tokens: u64, output_tokens: u64) -> f64 {
        let input_cost = (input_tokens as f64 / 1_000_000.0) * self.input_cost_per_million;
        let output_cost = (output_tokens as f64 / 1_000_000.0) * self.output_cost_per_million;
        input_cost + output_cost
    }
}

/// A single entry in the task board, tracking execution metrics for a task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskBoardEntry {
    /// Task identifier
    pub task_id: TaskId,
    /// Current state
    pub state: swell_core::TaskState,
    /// Assigned agent ID (if any)
    pub assigned_agent: Option<AgentId>,
    /// When task execution started (set when entering Executing state)
    pub started_at: Option<DateTime<Utc>>,
    /// When task completed (set when entering terminal state)
    pub completed_at: Option<DateTime<Utc>>,
    /// Total elapsed time in seconds (accumulated across pauses)
    pub elapsed_secs: u64,
    /// Total tokens consumed by this task
    pub tokens_used: u64,
    /// Estimated cost in USD
    pub estimated_cost_usd: f64,
    /// Token budget for this task
    pub token_budget: u64,
    /// Number of pause/resume cycles
    pub pause_count: u32,
    /// Last pause time (for calculating elapsed during pause)
    pub last_paused_at: Option<DateTime<Utc>>,
    /// Accumulated cost per model (for fallback tracking)
    #[serde(default)]
    pub cost_breakdown: HashMap<String, CostBreakdownEntry>,
}

/// Breakdown of costs per model
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CostBreakdownEntry {
    /// Input tokens used with this model
    pub input_tokens: u64,
    /// Output tokens used with this model
    pub output_tokens: u64,
    /// Total cost for this model
    pub cost_usd: f64,
}

impl TaskBoardEntry {
    /// Create a new entry for a task
    pub fn new(task_id: TaskId, token_budget: u64) -> Self {
        Self {
            task_id,
            state: swell_core::TaskState::Created,
            assigned_agent: None,
            started_at: None,
            completed_at: None,
            elapsed_secs: 0,
            tokens_used: 0,
            estimated_cost_usd: 0.0,
            token_budget,
            pause_count: 0,
            last_paused_at: None,
            cost_breakdown: HashMap::new(),
        }
    }

    /// Check if task is in a terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            swell_core::TaskState::Accepted
                | swell_core::TaskState::Rejected
                | swell_core::TaskState::Failed
                | swell_core::TaskState::Escalated
        )
    }

    /// Check if task is currently active (executing or validating)
    pub fn is_active(&self) -> bool {
        matches!(
            self.state,
            swell_core::TaskState::Executing | swell_core::TaskState::Validating
        )
    }

    /// Get elapsed time as Duration
    pub fn elapsed(&self) -> Duration {
        Duration::seconds(self.elapsed_secs as i64)
    }

    /// Get cost per token (returns 0 if no tokens used)
    pub fn cost_per_token(&self) -> f64 {
        if self.tokens_used == 0 {
            0.0
        } else {
            self.estimated_cost_usd / (self.tokens_used as f64)
        }
    }

    /// Get budget utilization percentage (0.0 to 1.0+)
    pub fn budget_utilization(&self) -> f64 {
        if self.token_budget == 0 {
            0.0
        } else {
            self.tokens_used as f64 / self.token_budget as f64
        }
    }
}

/// Task board tracking all tasks and their execution metrics
#[derive(Debug, Clone)]
pub struct TaskBoard {
    /// Entries indexed by task ID
    entries: HashMap<TaskId, TaskBoardEntry>,
    /// Cost model for calculating USD costs
    cost_model: CostModel,
}

impl TaskBoard {
    /// Create a new empty task board
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            cost_model: CostModel::default(),
        }
    }

    /// Create with a custom cost model
    pub fn with_cost_model(cost_model: CostModel) -> Self {
        Self {
            entries: HashMap::new(),
            cost_model,
        }
    }

    /// Add a new task to the board
    pub fn add_task(&mut self, task_id: TaskId, token_budget: u64) {
        let entry = TaskBoardEntry::new(task_id, token_budget);
        self.entries.insert(task_id, entry);
    }

    /// Get an entry by task ID
    pub fn get(&self, task_id: &TaskId) -> Option<&TaskBoardEntry> {
        self.entries.get(task_id)
    }

    /// Get a mutable entry by task ID
    pub fn get_mut(&mut self, task_id: &TaskId) -> Option<&mut TaskBoardEntry> {
        self.entries.get_mut(task_id)
    }

    /// Get all entries
    pub fn entries(&self) -> &HashMap<TaskId, TaskBoardEntry> {
        &self.entries
    }

    /// Update task state
    pub fn update_state(&mut self, task_id: &TaskId, state: swell_core::TaskState) {
        if let Some(entry) = self.entries.get_mut(task_id) {
            let now = Utc::now();

            // Handle state-specific logic
            match state {
                swell_core::TaskState::Executing => {
                    // Start timing if not already started
                    if entry.started_at.is_none() {
                        entry.started_at = Some(now);
                    }
                }
                swell_core::TaskState::Paused => {
                    entry.pause_count += 1;
                    entry.last_paused_at = Some(now);
                }
                swell_core::TaskState::Validating => {
                    // If we were paused, accumulate elapsed time
                    if let Some(last_paused) = entry.last_paused_at {
                        let paused_duration = now - last_paused;
                        entry.elapsed_secs += paused_duration.num_seconds() as u64;
                        entry.last_paused_at = None;
                    }
                }
                swell_core::TaskState::Accepted
                | swell_core::TaskState::Rejected
                | swell_core::TaskState::Failed
                | swell_core::TaskState::Escalated => {
                    // Finalize timing
                    entry.completed_at = Some(now);

                    // If we were executing/validating, add final elapsed time
                    if let Some(started) = entry.started_at {
                        let total_duration = now - started;
                        // Subtract any paused time
                        let paused_secs = entry.pause_count as i64 * 60; // Approximate 1 min per pause
                        entry.elapsed_secs = (total_duration.num_seconds() - paused_secs) as u64;
                    }
                }
                _ => {}
            }

            entry.state = state;
        }
    }

    /// Assign an agent to a task
    pub fn assign_agent(&mut self, task_id: &TaskId, agent_id: AgentId) {
        if let Some(entry) = self.entries.get_mut(task_id) {
            entry.assigned_agent = Some(agent_id);
        }
    }

    /// Record token usage for a task
    pub fn record_cost(&mut self, task_id: &TaskId, tokens: u64) {
        if let Some(entry) = self.entries.get_mut(task_id) {
            entry.tokens_used += tokens;

            // Estimate cost using default ratio (40% input, 60% output)
            let estimated = self
                .cost_model
                .calculate_cost((tokens as f64 * 0.4) as u64, (tokens as f64 * 0.6) as u64);
            entry.estimated_cost_usd += estimated;
        }
    }

    /// Record detailed cost breakdown per model
    pub fn record_model_cost(
        &mut self,
        task_id: &TaskId,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
    ) {
        if let Some(entry) = self.entries.get_mut(task_id) {
            let cost = self.cost_model.calculate_cost(input_tokens, output_tokens);

            entry.tokens_used += input_tokens + output_tokens;
            entry.estimated_cost_usd += cost;

            let breakdown = entry
                .cost_breakdown
                .entry(model.to_string())
                .or_insert_with(CostBreakdownEntry::default);
            breakdown.input_tokens += input_tokens;
            breakdown.output_tokens += output_tokens;
            breakdown.cost_usd += cost;
        }
    }

    /// Complete a task with final state
    pub fn complete_task(&mut self, task_id: &TaskId, final_state: swell_core::TaskState) {
        self.update_state(task_id, final_state);
    }

    /// Remove a task from the board
    pub fn remove_task(&mut self, task_id: &TaskId) -> Option<TaskBoardEntry> {
        self.entries.remove(task_id)
    }

    /// Get all tasks in a specific state
    pub fn get_by_state(&self, state: swell_core::TaskState) -> Vec<&TaskBoardEntry> {
        self.entries.values().filter(|e| e.state == state).collect()
    }

    /// Get all tasks assigned to a specific agent
    pub fn get_by_agent(&self, agent_id: &AgentId) -> Vec<&TaskBoardEntry> {
        self.entries
            .values()
            .filter(|e| e.assigned_agent == Some(*agent_id))
            .collect()
    }

    /// Get board statistics
    pub fn stats(&self) -> TaskBoardStats {
        let total = self.entries.len();
        let mut active = 0;
        let mut completed = 0;
        let mut total_elapsed = 0u64;
        let mut total_tokens = 0u64;
        let mut total_cost = 0.0;

        for entry in self.entries.values() {
            if entry.is_active() {
                active += 1;
            }
            if entry.is_terminal() {
                completed += 1;
            }
            total_elapsed += entry.elapsed_secs;
            total_tokens += entry.tokens_used;
            total_cost += entry.estimated_cost_usd;
        }

        TaskBoardStats {
            total_tasks: total,
            active_tasks: active,
            completed_tasks: completed,
            total_elapsed_secs: total_elapsed,
            total_tokens_used: total_tokens,
            total_estimated_cost_usd: total_cost,
            avg_cost_per_task: if completed > 0 {
                total_cost / completed as f64
            } else {
                0.0
            },
            avg_time_per_task_secs: if completed > 0 {
                total_elapsed / completed as u64
            } else {
                0
            },
        }
    }

    /// Get tasks sorted by cost (descending)
    pub fn top_by_cost(&self, limit: usize) -> Vec<&TaskBoardEntry> {
        let mut entries: Vec<_> = self.entries.values().collect();
        entries.sort_by(|a, b| {
            b.estimated_cost_usd
                .partial_cmp(&a.estimated_cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        entries.truncate(limit);
        entries
    }

    /// Get tasks sorted by elapsed time (descending)
    pub fn top_by_time(&self, limit: usize) -> Vec<&TaskBoardEntry> {
        let mut entries: Vec<_> = self.entries.values().collect();
        entries.sort_by(|a, b| b.elapsed_secs.cmp(&a.elapsed_secs));
        entries.truncate(limit);
        entries
    }

    /// Check if any task is over budget
    pub fn get_over_budget_tasks(&self) -> Vec<&TaskBoardEntry> {
        self.entries
            .values()
            .filter(|e| e.budget_utilization() > 1.0)
            .collect()
    }
}

impl Default for TaskBoard {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-safe wrapper for TaskBoard
pub type SharedTaskBoard = Arc<RwLock<TaskBoard>>;

/// Create a new shared task board
pub fn create_task_board() -> SharedTaskBoard {
    Arc::new(RwLock::new(TaskBoard::new()))
}

/// Statistics summary for the task board
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskBoardStats {
    pub total_tasks: usize,
    pub active_tasks: usize,
    pub completed_tasks: usize,
    pub total_elapsed_secs: u64,
    pub total_tokens_used: u64,
    pub total_estimated_cost_usd: f64,
    pub avg_cost_per_task: f64,
    pub avg_time_per_task_secs: u64,
}

impl std::fmt::Display for TaskBoardStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TaskBoardStats(total={}, active={}, completed={}, \
             elapsed={}s, tokens={}, cost=${:.4}, \
             avg_cost=${:.4}, avg_time={}s)",
            self.total_tasks,
            self.active_tasks,
            self.completed_tasks,
            self.total_elapsed_secs,
            self.total_tokens_used,
            self.total_estimated_cost_usd,
            self.avg_cost_per_task,
            self.avg_time_per_task_secs
        )
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_board_add_task() {
        let mut board = TaskBoard::new();
        let task_id = TaskId::new();

        board.add_task(task_id, 1_000_000);

        let entry = board.get(&task_id).unwrap();
        assert_eq!(entry.task_id, task_id);
        assert_eq!(entry.state, swell_core::TaskState::Created);
        assert!(entry.assigned_agent.is_none());
        assert_eq!(entry.tokens_used, 0);
    }

    #[test]
    fn test_task_board_update_state() {
        let mut board = TaskBoard::new();
        let task_id = TaskId::new();

        board.add_task(task_id, 1_000_000);
        board.update_state(&task_id, swell_core::TaskState::Executing);

        let entry = board.get(&task_id).unwrap();
        assert_eq!(entry.state, swell_core::TaskState::Executing);
        assert!(entry.started_at.is_some());
    }

    #[test]
    fn test_task_board_assign_agent() {
        let mut board = TaskBoard::new();
        let task_id = TaskId::new();
        let agent_id = AgentId::new();

        board.add_task(task_id, 1_000_000);
        board.assign_agent(&task_id, agent_id);

        let entry = board.get(&task_id).unwrap();
        assert_eq!(entry.assigned_agent, Some(agent_id));
    }

    #[test]
    fn test_task_board_record_cost() {
        let mut board = TaskBoard::new();
        let task_id = TaskId::new();

        board.add_task(task_id, 1_000_000);
        board.record_cost(&task_id, 1000);

        let entry = board.get(&task_id).unwrap();
        assert_eq!(entry.tokens_used, 1000);
        // Default cost model: $3/M input + $15/M output
        // 1000 tokens * 0.4 = 400 input, 0.6 = 600 output
        // Cost = (400/1M * 3) + (600/1M * 15) = 0.0012 + 0.009 = 0.0102
        assert!((entry.estimated_cost_usd - 0.0102).abs() < 0.001);
    }

    #[test]
    fn test_task_board_record_model_cost() {
        let mut board = TaskBoard::new();
        let task_id = TaskId::new();

        board.add_task(task_id, 1_000_000);
        board.record_model_cost(&task_id, "claude-sonnet-4-20250514", 400, 600);

        let entry = board.get(&task_id).unwrap();
        assert_eq!(entry.tokens_used, 1000);
        assert!(entry
            .cost_breakdown
            .contains_key("claude-sonnet-4-20250514"));

        let breakdown = entry
            .cost_breakdown
            .get("claude-sonnet-4-20250514")
            .unwrap();
        assert_eq!(breakdown.input_tokens, 400);
        assert_eq!(breakdown.output_tokens, 600);
    }

    #[test]
    fn test_task_board_complete_task() {
        let mut board = TaskBoard::new();
        let task_id = TaskId::new();

        board.add_task(task_id, 1_000_000);
        board.update_state(&task_id, swell_core::TaskState::Executing);
        board.complete_task(&task_id, swell_core::TaskState::Accepted);

        let entry = board.get(&task_id).unwrap();
        assert_eq!(entry.state, swell_core::TaskState::Accepted);
        assert!(entry.completed_at.is_some());
    }

    #[test]
    fn test_task_board_get_by_state() {
        let mut board = TaskBoard::new();
        let task1 = TaskId::new();
        let task2 = TaskId::new();
        let task3 = TaskId::new();

        board.add_task(task1, 1_000_000);
        board.add_task(task2, 1_000_000);
        board.add_task(task3, 1_000_000);

        board.update_state(&task1, swell_core::TaskState::Executing);
        board.update_state(&task2, swell_core::TaskState::Executing);
        board.update_state(&task3, swell_core::TaskState::Accepted);

        let executing = board.get_by_state(swell_core::TaskState::Executing);
        assert_eq!(executing.len(), 2);

        let accepted = board.get_by_state(swell_core::TaskState::Accepted);
        assert_eq!(accepted.len(), 1);
    }

    #[test]
    fn test_task_board_get_by_agent() {
        let mut board = TaskBoard::new();
        let task1 = TaskId::new();
        let task2 = TaskId::new();
        let agent1 = AgentId::new();
        let agent2 = AgentId::new();

        board.add_task(task1, 1_000_000);
        board.add_task(task2, 1_000_000);
        board.assign_agent(&task1, agent1);
        board.assign_agent(&task2, agent2);

        let agent1_tasks = board.get_by_agent(&agent1);
        assert_eq!(agent1_tasks.len(), 1);

        let agent2_tasks = board.get_by_agent(&agent2);
        assert_eq!(agent2_tasks.len(), 1);
    }

    #[test]
    fn test_task_board_stats() {
        let mut board = TaskBoard::new();
        let task1 = TaskId::new();
        let task2 = TaskId::new();

        board.add_task(task1, 1_000_000);
        board.add_task(task2, 1_000_000);

        board.update_state(&task1, swell_core::TaskState::Executing);
        board.record_cost(&task1, 1000);

        board.complete_task(&task2, swell_core::TaskState::Accepted);

        let stats = board.stats();
        assert_eq!(stats.total_tasks, 2);
        assert_eq!(stats.active_tasks, 1);
        assert_eq!(stats.completed_tasks, 1);
    }

    #[test]
    fn test_task_board_top_by_cost() {
        let mut board = TaskBoard::new();
        let task1 = TaskId::new();
        let task2 = TaskId::new();
        let task3 = TaskId::new();

        board.add_task(task1, 1_000_000);
        board.add_task(task2, 1_000_000);
        board.add_task(task3, 1_000_000);

        board.record_cost(&task1, 100);
        board.record_cost(&task2, 500);
        board.record_cost(&task3, 200);

        let top = board.top_by_cost(2);
        assert_eq!(top.len(), 2);
        // task2 has highest cost (500 tokens)
        assert_eq!(top[0].task_id, task2);
        assert_eq!(top[1].task_id, task3);
    }

    #[test]
    fn test_task_board_top_by_time() {
        let mut board = TaskBoard::new();
        let task1 = TaskId::new();
        let task2 = TaskId::new();

        board.add_task(task1, 1_000_000);
        board.add_task(task2, 1_000_000);

        // Manually set elapsed time (simulating completed tasks)
        board.update_state(&task1, swell_core::TaskState::Executing);
        if let Some(entry) = board.get_mut(&task1) {
            entry.elapsed_secs = 100;
        }

        board.complete_task(&task2, swell_core::TaskState::Accepted);
        if let Some(entry) = board.get_mut(&task2) {
            entry.elapsed_secs = 300;
        }

        let top = board.top_by_time(1);
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].task_id, task2);
    }

    #[test]
    fn test_task_board_over_budget() {
        let mut board = TaskBoard::new();
        let task1 = TaskId::new();
        let task2 = TaskId::new();

        board.add_task(task1, 100_000); // 100k budget
        board.add_task(task2, 1_000_000); // 1M budget

        board.record_cost(&task1, 150_000); // Over budget (150% used)
        board.record_cost(&task2, 500_000); // Under budget (50% used)

        let over_budget = board.get_over_budget_tasks();
        assert_eq!(over_budget.len(), 1);
        assert_eq!(over_budget[0].task_id, task1);
    }

    #[test]
    fn test_cost_model_calculation() {
        let model = CostModel::default();

        // 1M input + 1M output should cost $3 + $15 = $18
        let cost = model.calculate_cost(1_000_000, 1_000_000);
        assert!((cost - 18.0).abs() < 0.001);

        // 0 tokens should cost $0
        let cost = model.calculate_cost(0, 0);
        assert!((cost - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_cost_model_custom() {
        let model = CostModel {
            input_cost_per_million: 1.0,
            output_cost_per_million: 5.0,
        };

        // 1M input + 1M output should cost $1 + $5 = $6
        let cost = model.calculate_cost(1_000_000, 1_000_000);
        assert!((cost - 6.0).abs() < 0.001);
    }

    #[test]
    fn test_task_board_entry_is_terminal() {
        let entry = TaskBoardEntry::new(TaskId::new(), 1_000_000);
        assert!(!entry.is_terminal());

        let mut entry = entry;
        entry.state = swell_core::TaskState::Accepted;
        assert!(entry.is_terminal());

        entry.state = swell_core::TaskState::Failed;
        assert!(entry.is_terminal());
    }

    #[test]
    fn test_task_board_entry_is_active() {
        let entry = TaskBoardEntry::new(TaskId::new(), 1_000_000);
        assert!(!entry.is_active());

        let mut entry = entry;
        entry.state = swell_core::TaskState::Executing;
        assert!(entry.is_active());

        entry.state = swell_core::TaskState::Validating;
        assert!(entry.is_active());
    }

    #[test]
    fn test_task_board_remove_task() {
        let mut board = TaskBoard::new();
        let task_id = TaskId::new();

        board.add_task(task_id, 1_000_000);
        assert!(board.get(&task_id).is_some());

        let removed = board.remove_task(&task_id);
        assert!(removed.is_some());
        assert!(board.get(&task_id).is_none());
    }

    #[test]
    fn test_shared_task_board() {
        let board = create_task_board();
        let task_id = TaskId::new();

        // Write to board
        {
            let mut guard = board.blocking_write();
            guard.add_task(task_id, 1_000_000);
        }

        // Read from board
        {
            let guard = board.blocking_read();
            assert!(guard.get(&task_id).is_some());
        }
    }
}
