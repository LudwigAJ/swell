//! Git worktree pool manager for pre-provisioned worktrees per agent.
//!
//! This module provides:
//! - [`WorktreePool`] - Pool of pre-provisioned git worktrees
//! - [`Worktree`] - Individual worktree state
//! - [`WorktreeAllocation`] - Allocation record for tracking

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use swell_core::ids::TaskId;
use swell_core::ids::WorktreeId;
use swell_core::AgentId;
use swell_core::SwellError;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Individual worktree state
#[derive(Debug, Clone)]
pub struct Worktree {
    /// Unique identifier for this worktree
    pub id: WorktreeId,
    /// Path to the worktree directory
    pub path: PathBuf,
    /// Agent this worktree is allocated to (if any)
    pub agent_id: Option<AgentId>,
    /// Task this worktree is for (if any)
    pub task_id: Option<TaskId>,
    /// Whether this worktree is ready for use
    pub ready: bool,
    /// Branch name for this worktree
    pub branch: Option<String>,
}

impl Worktree {
    /// Create a new worktree in unallocated state
    fn new(id: WorktreeId, path: PathBuf) -> Self {
        Self {
            id,
            path,
            agent_id: None,
            task_id: None,
            ready: false,
            branch: None,
        }
    }

    /// Check if this worktree is available for allocation
    fn is_available(&self) -> bool {
        self.agent_id.is_none() && self.ready
    }

    /// Allocate this worktree to an agent
    fn allocate(&mut self, agent_id: AgentId, task_id: TaskId) {
        self.agent_id = Some(agent_id);
        self.task_id = Some(task_id);
        debug!(
            worktree_id = %self.id,
            agent_id = %agent_id,
            task_id = %task_id,
            "Worktree allocated"
        );
    }

    /// Release this worktree back to the pool
    fn release(&mut self) {
        if let Some(agent_id) = self.agent_id {
            debug!(
                worktree_id = %self.id,
                agent_id = %agent_id,
                "Worktree released"
            );
        }
        self.agent_id = None;
        self.task_id = None;
    }
}

/// A record of worktree allocation
#[derive(Debug, Clone)]
pub struct WorktreeAllocation {
    /// Worktree ID
    pub worktree_id: WorktreeId,
    /// Agent ID
    pub agent_id: AgentId,
    /// Task ID
    pub task_id: TaskId,
    /// Path to the worktree
    pub path: PathBuf,
    /// Branch name
    pub branch: String,
    /// When the allocation was made
    pub allocated_at: chrono::DateTime<chrono::Utc>,
}

impl WorktreeAllocation {
    fn new(worktree: &Worktree, agent_id: AgentId, task_id: TaskId) -> Option<Self> {
        let branch = worktree.branch.clone()?;
        Some(Self {
            worktree_id: worktree.id,
            agent_id,
            task_id,
            path: worktree.path.clone(),
            branch,
            allocated_at: chrono::Utc::now(),
        })
    }
}

/// Configuration for the worktree pool
#[derive(Debug, Clone)]
pub struct WorktreePoolConfig {
    /// Base repository path to create worktrees from
    pub base_repo: PathBuf,
    /// Directory to store worktrees
    pub worktree_dir: PathBuf,
    /// Number of worktrees to pre-provision
    pub pool_size: usize,
    /// Name prefix for worktree directories
    pub prefix: String,
}

impl Default for WorktreePoolConfig {
    fn default() -> Self {
        Self {
            base_repo: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            worktree_dir: PathBuf::from("/tmp/swell-worktrees"),
            pool_size: 3,
            prefix: "agent".to_string(),
        }
    }
}

/// A pool of pre-provisioned git worktrees for agent allocation.
///
/// The pool manages a set of worktrees that can be allocated to agents
/// for isolated work. Worktrees are created from a base repository and
/// cleaned up when released.
pub struct WorktreePool {
    config: WorktreePoolConfig,
    worktrees: Arc<RwLock<Vec<Worktree>>>,
    allocations: Arc<RwLock<AllocationsMap>>,
}

type AllocationsMap = HashMap<TaskId, WorktreeAllocation>;

impl WorktreePool {
    /// Create a new worktree pool with the given configuration
    pub fn new(config: WorktreePoolConfig) -> Self {
        Self {
            config,
            worktrees: Arc::new(RwLock::new(Vec::new())),
            allocations: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get the configuration
    pub fn config(&self) -> &WorktreePoolConfig {
        &self.config
    }

    /// Get the number of worktrees in the pool
    pub async fn pool_size(&self) -> usize {
        let worktrees = self.worktrees.read().await;
        worktrees.len()
    }

    /// Get the number of available (unallocated) worktrees
    pub async fn available_count(&self) -> usize {
        let worktrees = self.worktrees.read().await;
        worktrees.iter().filter(|w| w.is_available()).count()
    }

    /// Get the number of active allocations
    pub async fn allocation_count(&self) -> usize {
        let allocations = self.allocations.read().await;
        allocations.len()
    }

    /// Initialize the pool by pre-provisioning worktrees
    pub async fn initialize(&self) -> Result<(), SwellError> {
        info!(
            pool_size = self.config.pool_size,
            base_repo = %self.config.base_repo.display(),
            worktree_dir = %self.config.worktree_dir.display(),
            "Initializing worktree pool"
        );

        // Create the worktree directory if it doesn't exist
        tokio::fs::create_dir_all(&self.config.worktree_dir)
            .await
            .map_err(SwellError::IoError)?;

        // Create worktrees
        let mut worktrees = self.worktrees.write().await;
        for i in 0..self.config.pool_size {
            let worktree = self.create_worktree_internal(i).await?;
            worktrees.push(worktree);
        }

        info!(created = worktrees.len(), "Worktree pool initialized");

        Ok(())
    }

    /// Create a single worktree
    async fn create_worktree_internal(&self, index: usize) -> Result<Worktree, SwellError> {
        let id = WorktreeId::new();
        let name = format!("{}-{}", self.config.prefix, index);
        let path = self.config.worktree_dir.join(&name);
        let path_str = path.to_string_lossy().to_string();

        // Create worktree using git worktree add
        let result = tokio::process::Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                &format!("worktree/{}/{}", self.config.prefix, id),
                &path_str,
            ])
            .current_dir(&self.config.base_repo)
            .output()
            .await;

        match result {
            Ok(output) if output.status.success() => {
                debug!(path = %path.display(), "Created worktree");
                let mut worktree = Worktree::new(id, path);
                worktree.ready = true;
                worktree.branch = Some(format!("worktree/{}/{}", self.config.prefix, id));
                Ok(worktree)
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                // If worktree already exists, just use it
                if stderr.contains("already exists") {
                    let mut worktree = Worktree::new(id, path);
                    worktree.ready = true;
                    worktree.branch = Some(format!("worktree/{}/{}", self.config.prefix, id));
                    Ok(worktree)
                } else {
                    warn!(error = %stderr, "Failed to create worktree, marking as not ready");
                    let mut worktree = Worktree::new(id, path);
                    worktree.ready = false;
                    Ok(worktree)
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to create worktree, marking as not ready");
                let mut worktree = Worktree::new(id, path);
                worktree.ready = false;
                Ok(worktree)
            }
        }
    }

    /// Allocate a worktree to an agent for a specific task
    pub async fn allocate(
        &self,
        agent_id: AgentId,
        task_id: TaskId,
    ) -> Result<WorktreeAllocation, SwellError> {
        // Find an available worktree
        let worktree_id = {
            let mut worktrees = self.worktrees.write().await;
            let available = worktrees
                .iter_mut()
                .find(|w| w.is_available())
                .ok_or_else(|| {
                    SwellError::ToolExecutionFailed("No available worktrees in pool".to_string())
                })?;

            available.allocate(agent_id, task_id);
            available.id
        };

        // Get the allocated worktree
        let worktree = {
            let worktrees = self.worktrees.read().await;
            worktrees.iter().find(|w| w.id == worktree_id).cloned()
        };

        let worktree = worktree
            .ok_or_else(|| SwellError::ToolExecutionFailed("Worktree not found".to_string()))?;

        // Create allocation record
        let allocation =
            WorktreeAllocation::new(&worktree, agent_id, task_id).ok_or_else(|| {
                SwellError::ToolExecutionFailed("Worktree missing branch".to_string())
            })?;

        // Store allocation
        {
            let mut allocations = self.allocations.write().await;
            allocations.insert(task_id, allocation.clone());
        }

        info!(
            worktree_id = %allocation.worktree_id,
            agent_id = %agent_id,
            task_id = %task_id,
            path = %allocation.path.display(),
            branch = %allocation.branch,
            "Worktree allocated to agent"
        );

        Ok(allocation)
    }

    /// Release a worktree allocation after task completion
    pub async fn release(&self, task_id: TaskId) -> Result<(), SwellError> {
        let allocation = {
            let mut allocations = self.allocations.write().await;
            allocations.remove(&task_id)
        };

        if let Some(allocation) = allocation {
            // Release the worktree back to the pool
            let mut worktrees = self.worktrees.write().await;
            if let Some(worktree) = worktrees
                .iter_mut()
                .find(|w| w.id == allocation.worktree_id)
            {
                worktree.release();
            }

            // Cleanup the worktree (remove the branch and worktree)
            self.cleanup_worktree(&allocation).await?;

            // Mark worktree as not ready after cleanup since it no longer exists
            if let Some(worktree) = worktrees
                .iter_mut()
                .find(|w| w.id == allocation.worktree_id)
            {
                worktree.ready = false;
            }

            info!(
                worktree_id = %allocation.worktree_id,
                task_id = %task_id,
                "Worktree released"
            );
        }

        Ok(())
    }

    /// Release worktree by agent ID
    pub async fn release_by_agent(&self, agent_id: AgentId) -> Result<(), SwellError> {
        let task_ids: Vec<TaskId> = {
            let allocations = self.allocations.read().await;
            allocations
                .iter()
                .filter(|(_, a)| a.agent_id == agent_id)
                .map(|(task_id, _)| *task_id)
                .collect()
        };

        for task_id in task_ids {
            self.release(task_id).await?;
        }

        Ok(())
    }

    /// Cleanup a worktree (remove branch and worktree directory)
    async fn cleanup_worktree(&self, allocation: &WorktreeAllocation) -> Result<(), SwellError> {
        let path_str = allocation.path.to_string_lossy().to_string();

        // Remove the worktree using git worktree remove
        let result = tokio::process::Command::new("git")
            .args(["worktree", "remove", "--force", &path_str])
            .current_dir(&self.config.base_repo)
            .output()
            .await;

        match result {
            Ok(output) if output.status.success() => {
                debug!(path = %allocation.path.display(), "Worktree removed");
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                // It's okay if the worktree is already gone
                if !stderr.contains("not found") && !stderr.contains("No such file") {
                    warn!(error = %stderr, "Failed to remove worktree, attempting cleanup");
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to remove worktree during cleanup");
            }
        }

        // Remove the branch
        let branch_name = &allocation.branch;
        let result = tokio::process::Command::new("git")
            .args(["branch", "-D", branch_name])
            .current_dir(&self.config.base_repo)
            .output()
            .await;

        if let Ok(output) = result {
            if output.status.success() {
                debug!(branch = %branch_name, "Branch deleted");
            }
        }

        Ok(())
    }

    /// Get allocation info for a task
    pub async fn get_allocation(&self, task_id: TaskId) -> Option<WorktreeAllocation> {
        let allocations = self.allocations.read().await;
        allocations.get(&task_id).cloned()
    }

    /// Get all current allocations
    pub async fn list_allocations(&self) -> Vec<WorktreeAllocation> {
        let allocations = self.allocations.read().await;
        allocations.values().cloned().collect()
    }

    /// Get all worktrees in the pool
    pub async fn list_worktrees(&self) -> Vec<Worktree> {
        let worktrees = self.worktrees.read().await;
        worktrees.clone()
    }

    /// Get a specific worktree by ID
    pub async fn get_worktree(&self, id: WorktreeId) -> Option<Worktree> {
        let worktrees = self.worktrees.read().await;
        worktrees.iter().find(|w| w.id == id).cloned()
    }

    /// Check if a task has an allocated worktree
    pub async fn has_allocation(&self, task_id: TaskId) -> bool {
        let allocations = self.allocations.read().await;
        allocations.contains_key(&task_id)
    }
}

impl Clone for WorktreePool {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            worktrees: self.worktrees.clone(),
            allocations: self.allocations.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_test_pool() -> WorktreePool {
        let dir = tempdir().unwrap();
        let config = WorktreePoolConfig {
            base_repo: dir.path().to_path_buf(),
            worktree_dir: dir.path().join("worktrees"),
            pool_size: 2,
            prefix: "test".to_string(),
        };
        WorktreePool::new(config)
    }

    #[tokio::test]
    async fn test_worktree_pool_creation() {
        let pool = create_test_pool();
        assert_eq!(pool.config().pool_size, 2);
        assert_eq!(pool.allocation_count().await, 0);
    }

    #[tokio::test]
    async fn test_worktree_state() {
        let mut worktree = Worktree::new(WorktreeId::new(), PathBuf::from("/tmp/test"));
        worktree.ready = true;
        assert!(worktree.is_available());
        assert!(worktree.agent_id.is_none());

        let agent_id = AgentId::new();
        let task_id = TaskId::new();
        worktree.allocate(agent_id, task_id);

        assert!(!worktree.is_available());
        assert_eq!(worktree.agent_id, Some(agent_id));
        assert_eq!(worktree.task_id, Some(task_id));
    }

    #[tokio::test]
    async fn test_worktree_release() {
        let mut worktree = Worktree::new(WorktreeId::new(), PathBuf::from("/tmp/test"));
        worktree.ready = true;

        let agent_id = AgentId::new();
        let task_id = TaskId::new();
        worktree.allocate(agent_id, task_id);
        assert!(!worktree.is_available());

        worktree.release();
        assert!(worktree.is_available());
        assert!(worktree.agent_id.is_none());
        assert!(worktree.task_id.is_none());
    }

    #[tokio::test]
    async fn test_allocation_record() {
        let mut worktree = Worktree::new(WorktreeId::new(), PathBuf::from("/tmp/test"));
        worktree.branch = Some("test-branch".to_string());
        worktree.ready = true;

        let agent_id = AgentId::new();
        let task_id = TaskId::new();
        worktree.allocate(agent_id, task_id);

        let allocation = WorktreeAllocation::new(&worktree, agent_id, task_id);
        assert!(allocation.is_some());

        let allocation = allocation.unwrap();
        assert_eq!(allocation.agent_id, agent_id);
        assert_eq!(allocation.task_id, task_id);
        assert_eq!(allocation.branch, "test-branch");
    }

    #[tokio::test]
    async fn test_pool_tracking() {
        let pool = create_test_pool();
        assert_eq!(pool.available_count().await, 0);
        assert_eq!(pool.allocation_count().await, 0);
    }

    #[tokio::test]
    async fn test_has_allocation() {
        let pool = create_test_pool();
        let task_id = TaskId::new();
        assert!(!pool.has_allocation(task_id).await);
    }

    #[tokio::test]
    async fn test_list_worktrees_empty() {
        let pool = create_test_pool();
        let worktrees = pool.list_worktrees().await;
        assert!(worktrees.is_empty());
    }

    #[tokio::test]
    async fn test_list_allocations_empty() {
        let pool = create_test_pool();
        let allocations = pool.list_allocations().await;
        assert!(allocations.is_empty());
    }

    #[tokio::test]
    async fn test_worktree_config_default() {
        let config = WorktreePoolConfig::default();
        assert_eq!(config.pool_size, 3);
        assert_eq!(config.prefix, "agent");
    }
}
