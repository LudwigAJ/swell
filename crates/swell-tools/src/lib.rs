//! Tool registry and execution for SWELL.
//!
//! This crate provides:
//! - [`ToolRegistry`] - central registry for all tools
//! - [`ToolExecutor`] - executes tools with permission enforcement
//! - Built-in tools: file I/O, git, shell execution
//! - MCP client for external tool servers
//! - [`WorktreePool`] - git worktree pool for agent isolation

pub mod branch_strategy;
pub mod executor;
pub mod mcp;
pub mod registry;
pub mod tools;
pub mod worktree_pool;

pub use branch_strategy::{
    BranchRequest, BranchResult, BranchStrategy, BranchStrategyConfig, BranchStrategyError,
};
pub use executor::ToolExecutor;
pub use registry::{ToolRegistration, ToolRegistry};
pub use worktree_pool::{WorktreeAllocation, WorktreePool, WorktreePoolConfig};

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_registry_empty() {
        let registry = ToolRegistry::new();
        assert!(registry.list().await.is_empty());
    }

    #[tokio::test]
    async fn test_registry_registration() {
        let registry = ToolRegistry::new();
        registry.register(tools::ReadFileTool::new()).await;
        assert_eq!(registry.list().await.len(), 1);
    }
}
