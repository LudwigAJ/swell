//! Tool registry and execution for SWELL.
//!
//! This crate provides:
//! - [`ToolRegistry`] - central registry for all tools
//! - [`ToolExecutor`] - executes tools with permission enforcement
//! - Built-in tools: file I/O, git, shell execution
//! - MCP client for external tool servers
//! - [`WorktreePool`] - git worktree pool for agent isolation
//! - [`CommitStrategy`] - atomic commits with metadata trailers
//! - [`PrCreator`] - PR creation with metadata, evidence, and labels

pub mod branch_strategy;
pub mod commit_strategy;
pub mod conflict_resolution;
pub mod executor;
pub mod mcp;
pub mod pr_creation;
pub mod registry;
pub mod tools;
pub mod worktree_pool;

pub use branch_strategy::{
    BranchRequest, BranchResult, BranchStrategy, BranchStrategyConfig, BranchStrategyError,
};
pub use commit_strategy::{
    CommitMetadata, CommitRequest, CommitResult, CommitStrategy, CommitStrategyError,
};
pub use conflict_resolution::{
    ConflictDetectionResult, ConflictHunk, ConflictInfo, ConflictResolutionError, ConflictResolver,
    ConflictResolverConfig, FileOwner, ResolutionResult, ResolutionStrategy,
};
pub use executor::ToolExecutor;
pub use pr_creation::{
    EvidenceSummary, PrCreationError, PrCreator, PrCreatorConfig, PrLabel, PrMetadata, PrResult,
};
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
