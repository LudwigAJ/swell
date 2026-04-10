//! Tool registry and execution for SWELL.
//!
//! This crate provides:
//! - [`ToolRegistry`] - central registry for all tools
//! - [`ToolExecutor`] - executes tools with permission enforcement
//! - Built-in tools: file I/O, git, shell execution
//! - MCP client for external tool servers
//! - [`WorktreePool`] - git worktree pool for agent isolation
//! - [`WorktreeIsolation`] - per-worktree environment isolation with separate PATH, env vars
//! - [`CommitStrategy`] - atomic commits with metadata trailers
//! - [`PrCreator`] - PR creation with metadata, evidence, and labels

pub mod branch_strategy;
pub mod commit_strategy;
pub mod conflict_resolution;
pub mod credential_proxy;
pub mod executor;
pub mod hybrid;
pub mod mcp;
pub mod os_sandbox;
pub mod pr_creation;
pub mod registry;
pub mod tools;
pub mod worktree_isolation;
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
pub use credential_proxy::{
    AccessToken, Credential, CredentialProxy, CredentialProxyError, CredentialProvider,
    CredentialScope, EnvCredentialProvider,
};
pub use executor::ToolExecutor;
pub use hybrid::{
    ExecutorInput, HybridConfig, HybridExecutor, LocalExecutor, RemoteExecutor, RiskClass,
    ToolExecutorTrait,
};
pub use os_sandbox::{
    detect_available_sandbox, detect_available_sandbox_sync, BubblewrapSandbox, FilesystemPermission,
    LandlockSandbox, NetworkPolicy, OsSandbox, OsSandboxConfig, PlatformSandbox,
    SandboxAvailability, SandboxType, SeatbeltSandbox,
};
pub use pr_creation::{
    EvidenceSummary, PrCreationError, PrCreator, PrCreatorConfig, PrLabel, PrMetadata, PrResult,
};
pub use registry::{ToolRegistration, ToolRegistry};
pub use worktree_isolation::{WorktreeIsolation, WorktreeIsolationConfig};
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
