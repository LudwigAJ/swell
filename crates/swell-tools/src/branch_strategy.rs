//! Git branch strategy implementation.
//!
//! This module provides:
//! - [`BranchStrategy`] - Deterministic branch naming with protection rules
//! - Branch naming convention: `agent/<task-id>/<description>`
//! - Main branch protection (never work on main)
//! - Branch limit enforcement (20 per run)

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use swell_core::ids::TaskId;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Configuration for branch naming
#[derive(Debug, Clone)]
pub struct BranchStrategyConfig {
    /// Prefix for branch names (default: "agent")
    pub branch_prefix: String,
    /// Maximum number of active branches per run (default: 20)
    pub max_active_branches: usize,
    /// Protected branches that should never be modified (default: ["main", "master"])
    pub protected_branches: Vec<String>,
}

impl Default for BranchStrategyConfig {
    fn default() -> Self {
        Self {
            branch_prefix: "agent".to_string(),
            max_active_branches: 20,
            protected_branches: vec!["main".to_string(), "master".to_string()],
        }
    }
}

impl BranchStrategyConfig {
    /// Create a new config with custom values
    pub fn new(branch_prefix: String, max_active_branches: usize) -> Self {
        Self {
            branch_prefix,
            max_active_branches,
            protected_branches: vec!["main".to_string(), "master".to_string()],
        }
    }

    /// Create config from settings.json git section
    pub fn from_settings(
        branch_prefix: Option<String>,
        max_active_branches: Option<usize>,
    ) -> Self {
        Self {
            branch_prefix: branch_prefix.unwrap_or_else(|| "agent".to_string()),
            max_active_branches: max_active_branches.unwrap_or(20),
            protected_branches: vec!["main".to_string(), "master".to_string()],
        }
    }
}

/// Branch creation request
#[derive(Debug, Clone)]
pub struct BranchRequest {
    /// Task ID for the branch
    pub task_id: TaskId,
    /// Description for the branch (will be sanitized for branch name)
    pub description: String,
    /// Optional base branch (defaults to main)
    pub base_branch: Option<String>,
}

impl BranchRequest {
    /// Create a new branch request
    pub fn new(task_id: TaskId, description: String) -> Self {
        Self {
            task_id,
            description,
            base_branch: None,
        }
    }

    /// Create a new branch request with a specific base branch
    pub fn with_base_branch(mut self, base_branch: String) -> Self {
        self.base_branch = Some(base_branch);
        self
    }
}

/// Branch creation result
#[derive(Debug, Clone)]
pub struct BranchResult {
    /// The full branch name created
    pub branch_name: String,
    /// The task ID associated with this branch
    pub task_id: TaskId,
    /// Whether this was a new branch or existing
    pub is_new: bool,
}

/// Errors that can occur during branch operations
#[derive(Debug, Clone, thiserror::Error)]
pub enum BranchStrategyError {
    #[error("Branch limit exceeded: {0} active branches (max: {1})")]
    BranchLimitExceeded(usize, usize),

    #[error("Protected branch '{0}' cannot be modified")]
    ProtectedBranch(String),

    #[error("Invalid branch name: {0}")]
    InvalidBranchName(String),

    #[error("Branch '{0}' not found")]
    BranchNotFound(String),

    #[error("Git operation failed: {0}")]
    GitFailed(String),
}

/// Thread-safe branch strategy for managing git branches with naming conventions and limits.
#[derive(Debug, Clone)]
pub struct BranchStrategy {
    config: BranchStrategyConfig,
    /// Tracks active branches created by this strategy in the current run
    active_branches: Arc<RwLock<HashMap<String, TaskId>>>,
}

impl BranchStrategy {
    /// Create a new branch strategy with default configuration
    pub fn new() -> Self {
        Self {
            config: BranchStrategyConfig::default(),
            active_branches: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new branch strategy with custom configuration
    pub fn with_config(config: BranchStrategyConfig) -> Self {
        Self {
            config,
            active_branches: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new branch strategy from settings
    pub fn from_settings(
        branch_prefix: Option<String>,
        max_active_branches: Option<usize>,
    ) -> Self {
        Self {
            config: BranchStrategyConfig::from_settings(branch_prefix, max_active_branches),
            active_branches: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get the current configuration
    pub fn config(&self) -> &BranchStrategyConfig {
        &self.config
    }

    /// Get the number of currently active branches
    pub async fn active_count(&self) -> usize {
        let branches = self.active_branches.read().await;
        branches.len()
    }

    /// Get all active branch names
    pub async fn active_branches(&self) -> Vec<String> {
        let branches = self.active_branches.read().await;
        branches.keys().cloned().collect()
    }

    /// Check if a branch name is valid according to git conventions
    pub fn is_valid_branch_name(name: &str) -> bool {
        if name.is_empty() {
            return false;
        }

        // Git branch names cannot contain whitespace, special git refs, or control chars
        let invalid_chars = [' ', '\t', '\n', '^', '~', ':', '?', '*', '[', '\\'];
        if name.chars().any(|c| invalid_chars.contains(&c)) {
            return false;
        }

        // Cannot start with '/' or end with '/'
        if name.starts_with('/') || name.ends_with('/') {
            return false;
        }

        // Cannot contain '..'
        if name.contains("..") {
            return false;
        }

        // Cannot be exactly '.' or '..'
        if name == "." || name == ".." {
            return false;
        }

        // Cannot contain lock refs
        if name.contains("/HEAD") || name.contains("/locked") {
            return false;
        }

        true
    }

    /// Sanitize a description for use in a branch name
    fn sanitize_description(description: &str) -> String {
        description
            .chars()
            .map(|c| {
                // Replace invalid branch name chars with hyphens
                match c {
                    ' ' | '\t' | '\n' => '-',
                    '^' | '~' | ':' | '?' | '*' | '[' | '\\' | '/' | '#' | '|' | '&' | ';'
                    | '"' | '\'' | '<' | '>' => '-',
                    _ => c,
                }
            })
            .collect::<String>()
            // Collapse multiple hyphens into one
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-")
            // Truncate if too long (git branch names have limits, ~100 chars is safe)
            .chars()
            .take(80)
            .collect()
    }

    /// Generate a deterministic branch name for a task
    /// Format: agent/<task-id>/<sanitized-description>
    pub fn generate_branch_name(&self, task_id: TaskId, description: &str) -> String {
        let sanitized = Self::sanitize_description(description);
        let task_id_str = task_id.to_string();
        let task_short = task_id_str.split('-').next().unwrap_or("task");
        format!("{}/{}/{}", self.config.branch_prefix, task_short, sanitized)
    }

    /// Check if a branch is protected (main, master, etc.)
    pub fn is_protected_branch(&self, branch_name: &str) -> bool {
        self.config
            .protected_branches
            .iter()
            .any(|p| branch_name == p || branch_name.starts_with(&format!("{}/", p)))
    }

    /// Check if we're at the branch limit
    pub async fn is_at_limit(&self) -> bool {
        self.active_count().await >= self.config.max_active_branches
    }

    /// Validate a branch request before creation
    pub async fn validate(&self, request: &BranchRequest) -> Result<(), BranchStrategyError> {
        // Check branch limit
        if self.is_at_limit().await {
            let count = self.active_count().await;
            return Err(BranchStrategyError::BranchLimitExceeded(
                count,
                self.config.max_active_branches,
            ));
        }

        // Check if base branch is protected
        let base_branch = request.base_branch.as_deref().unwrap_or("main");
        if self.is_protected_branch(base_branch) {
            warn!(branch = %base_branch, "Attempted to work on protected branch");
            return Err(BranchStrategyError::ProtectedBranch(
                base_branch.to_string(),
            ));
        }

        // Generate and validate the proposed branch name
        let proposed = self.generate_branch_name(request.task_id, &request.description);
        if !Self::is_valid_branch_name(&proposed) {
            return Err(BranchStrategyError::InvalidBranchName(proposed));
        }

        Ok(())
    }

    /// Register a branch as active (call after successful creation)
    pub async fn register_branch(&self, branch_name: String, task_id: TaskId) {
        let mut branches = self.active_branches.write().await;
        branches.insert(branch_name.clone(), task_id);
        debug!(
            branch = %branch_name,
            task_id = %task_id,
            active_count = branches.len(),
            "Branch registered"
        );
    }

    /// Unregister a branch (call after branch is merged/cleaned up)
    pub async fn unregister_branch(&self, branch_name: &str) {
        let mut branches = self.active_branches.write().await;
        if branches.remove(branch_name).is_some() {
            debug!(
                branch = %branch_name,
                remaining = branches.len(),
                "Branch unregistered"
            );
        }
    }

    /// Check if a branch is tracked by this strategy
    pub async fn is_tracked(&self, branch_name: &str) -> bool {
        let branches = self.active_branches.read().await;
        branches.contains_key(branch_name)
    }

    /// Get the task ID associated with a branch
    pub async fn get_task_id(&self, branch_name: &str) -> Option<TaskId> {
        let branches = self.active_branches.read().await;
        branches.get(branch_name).copied()
    }

    /// Clear all tracked branches (for run reset)
    pub async fn reset(&self) {
        let count = {
            let branches = self.active_branches.read().await;
            branches.len()
        };
        if count > 0 {
            info!(cleared = count, "Branch strategy reset");
        }
        let mut branches = self.active_branches.write().await;
        branches.clear();
    }

    /// Get proposed branch name and register it (reserve slot)
    /// Use this when you intend to create the branch
    pub async fn propose_branch(
        &self,
        request: &BranchRequest,
    ) -> Result<String, BranchStrategyError> {
        self.validate(request).await?;
        let branch_name = self.generate_branch_name(request.task_id, &request.description);
        // Register to consume a slot in the limit
        self.register_branch(branch_name.clone(), request.task_id)
            .await;
        Ok(branch_name)
    }

    /// Create a branch with the strategy's naming convention
    /// This performs validation, generates the name, and registers it
    pub async fn create_branch(
        &self,
        request: BranchRequest,
        cwd: &Path,
    ) -> Result<BranchResult, BranchStrategyError> {
        // Validate the request
        self.validate(&request).await?;

        // Generate branch name
        let branch_name = self.generate_branch_name(request.task_id, &request.description);

        // Determine base branch
        let base = request.base_branch.as_deref().unwrap_or("main");

        // Execute git branch creation
        let output = tokio::process::Command::new("git")
            .args(["checkout", "-b", &branch_name])
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| BranchStrategyError::GitFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);

            // Check if branch already exists
            if stderr.contains("already exists") || stderr.contains("A branch named") {
                // Branch already exists, just register it
                self.register_branch(branch_name.clone(), request.task_id)
                    .await;
                return Ok(BranchResult {
                    branch_name,
                    task_id: request.task_id,
                    is_new: false,
                });
            }

            return Err(BranchStrategyError::GitFailed(stderr.to_string()));
        }

        // Register the new branch
        self.register_branch(branch_name.clone(), request.task_id)
            .await;

        info!(
            branch = %branch_name,
            task_id = %request.task_id,
            base = %base,
            "Branch created via strategy"
        );

        Ok(BranchResult {
            branch_name,
            task_id: request.task_id,
            is_new: true,
        })
    }

    /// Delete a branch (with protection check)
    pub async fn delete_branch(
        &self,
        branch_name: &str,
        cwd: &Path,
    ) -> Result<(), BranchStrategyError> {
        // Check protection
        if self.is_protected_branch(branch_name) {
            return Err(BranchStrategyError::ProtectedBranch(
                branch_name.to_string(),
            ));
        }

        // Execute git branch deletion
        let output = tokio::process::Command::new("git")
            .args(["branch", "-D", branch_name])
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| BranchStrategyError::GitFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("not found") || stderr.contains("does not exist") {
                return Err(BranchStrategyError::BranchNotFound(branch_name.to_string()));
            }
            return Err(BranchStrategyError::GitFailed(stderr.to_string()));
        }

        // Unregister if tracked
        self.unregister_branch(branch_name).await;

        info!(branch = %branch_name, "Branch deleted via strategy");

        Ok(())
    }
}

impl Default for BranchStrategy {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_branch_strategy_default_config() {
        let strategy = BranchStrategy::new();
        assert_eq!(strategy.config().branch_prefix, "agent");
        assert_eq!(strategy.config().max_active_branches, 20);
        assert_eq!(strategy.active_count().await, 0);
    }

    #[tokio::test]
    async fn test_branch_strategy_custom_config() {
        let config = BranchStrategyConfig::new("feature".to_string(), 10);
        let strategy = BranchStrategy::with_config(config);
        assert_eq!(strategy.config().branch_prefix, "feature");
        assert_eq!(strategy.config().max_active_branches, 10);
    }

    #[tokio::test]
    async fn test_generate_branch_name() {
        let strategy = BranchStrategy::new();
        let task_id = TaskId::nil();

        let name = strategy.generate_branch_name(task_id, "fix bug in auth");
        assert_eq!(name, "agent/550e8400/fix-bug-in-auth");
    }

    #[tokio::test]
    async fn test_generate_branch_name_special_chars() {
        let strategy = BranchStrategy::new();
        let task_id = TaskId::new();

        let name = strategy.generate_branch_name(task_id, "fix: auth issue #123");
        assert_eq!(
            name,
            format!(
                "agent/{}/fix-auth-issue-123",
                task_id.to_string().split('-').next().unwrap()
            )
        );

        let name2 = strategy.generate_branch_name(task_id, "auth module | refactor");
        assert!(
            name2.contains("auth-module-refactor") || name2.contains("auth-module-----refactor")
        );
    }

    #[tokio::test]
    async fn test_is_protected_branch() {
        let strategy = BranchStrategy::new();

        assert!(strategy.is_protected_branch("main"));
        assert!(strategy.is_protected_branch("master"));
        assert!(strategy.is_protected_branch("main/test"));

        assert!(!strategy.is_protected_branch("agent/123/task"));
        assert!(!strategy.is_protected_branch("feature/new"));
    }

    #[test]
    fn test_is_valid_branch_name() {
        assert!(BranchStrategy::is_valid_branch_name("agent/123/fix-bug"));
        assert!(BranchStrategy::is_valid_branch_name("feature-new"));
        assert!(BranchStrategy::is_valid_branch_name("bugfix_123"));

        assert!(!BranchStrategy::is_valid_branch_name(""));
        assert!(!BranchStrategy::is_valid_branch_name(" "));
        assert!(!BranchStrategy::is_valid_branch_name("has space"));
        assert!(!BranchStrategy::is_valid_branch_name("has\ttab"));
        assert!(!BranchStrategy::is_valid_branch_name(".."));
        assert!(!BranchStrategy::is_valid_branch_name("."));
        assert!(!BranchStrategy::is_valid_branch_name("/starts-with-slash"));
        assert!(!BranchStrategy::is_valid_branch_name("ends-with-slash/"));
    }

    #[test]
    fn test_sanitize_description() {
        assert_eq!(BranchStrategy::sanitize_description("fix bug"), "fix-bug");
        assert_eq!(
            BranchStrategy::sanitize_description("fix: auth issue"),
            "fix-auth-issue"
        );
        assert_eq!(
            BranchStrategy::sanitize_description("has   spaces"),
            "has-spaces"
        );
        assert_eq!(
            BranchStrategy::sanitize_description("special ^chars~"),
            "special-chars"
        );
    }

    #[tokio::test]
    async fn test_register_unregister_branch() {
        let strategy = BranchStrategy::new();
        let task_id = TaskId::new();
        let branch_name = "agent/test/branch";

        assert_eq!(strategy.active_count().await, 0);

        strategy
            .register_branch(branch_name.to_string(), task_id)
            .await;
        assert_eq!(strategy.active_count().await, 1);
        assert!(strategy.is_tracked(branch_name).await);
        assert_eq!(strategy.get_task_id(branch_name).await, Some(task_id));

        strategy.unregister_branch(branch_name).await;
        assert_eq!(strategy.active_count().await, 0);
        assert!(!strategy.is_tracked(branch_name).await);
    }

    #[tokio::test]
    async fn test_branch_limit() {
        let config = BranchStrategyConfig::new("test".to_string(), 2);
        let strategy = BranchStrategy::with_config(config);

        let task1 = TaskId::new();
        let task2 = TaskId::new();
        let task3 = TaskId::new();

        let req1 =
            BranchRequest::new(task1, "task 1".to_string()).with_base_branch("develop".to_string());
        let req2 =
            BranchRequest::new(task2, "task 2".to_string()).with_base_branch("develop".to_string());
        let req3 =
            BranchRequest::new(task3, "task 3".to_string()).with_base_branch("develop".to_string());

        // First two should succeed
        strategy.propose_branch(&req1).await.unwrap();
        strategy.propose_branch(&req2).await.unwrap();

        // Third should fail
        let result = strategy.propose_branch(&req3).await;
        assert!(matches!(
            result,
            Err(BranchStrategyError::BranchLimitExceeded(2, 2))
        ));
    }

    #[tokio::test]
    async fn test_reset() {
        let strategy = BranchStrategy::new();
        let task_id = TaskId::new();

        strategy
            .register_branch("test/branch".to_string(), task_id)
            .await;
        assert_eq!(strategy.active_count().await, 1);

        strategy.reset().await;
        assert_eq!(strategy.active_count().await, 0);
    }

    #[tokio::test]
    async fn test_propose_branch_validates() {
        let config = BranchStrategyConfig::new("test".to_string(), 1);
        let strategy = BranchStrategy::with_config(config);

        let task_id = TaskId::new();
        let req = BranchRequest::new(task_id, "first task".to_string())
            .with_base_branch("develop".to_string());

        // Should succeed
        let branch = strategy.propose_branch(&req).await.unwrap();
        assert!(branch.contains("first-task"));

        // Second should fail limit check
        let task2 = TaskId::new();
        let req2 = BranchRequest::new(task2, "second task".to_string())
            .with_base_branch("develop".to_string());
        let result = strategy.propose_branch(&req2).await;
        assert!(matches!(
            result,
            Err(BranchStrategyError::BranchLimitExceeded(1, 1))
        ));
    }
}
