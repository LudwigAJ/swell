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
//! - Cedar policy engine for formally verifiable tool access control

pub mod branch_strategy;
pub mod cedar_policy;
pub mod commit_strategy;
pub mod conflict_resolution;
pub mod credential_proxy;
pub mod egress;
pub mod executor;
pub mod hybrid;
pub mod mcp;
pub mod os_sandbox;
pub mod pr_creation;
pub mod registry;
pub mod auto_masking;
pub mod secret_scanning;
pub mod tools;
pub mod vault;
pub mod worktree_isolation;
pub mod worktree_pool;

pub use auto_masking::{
    AutoMasker, MaskingConfig, MaskingResult, MaskingStats, SecretPattern, MaskSecrets,
};
pub use branch_strategy::{
    BranchRequest, BranchResult, BranchStrategy, BranchStrategyConfig, BranchStrategyError,
};
pub use cedar_policy::{
    CedarDecision, CedarError, CedarPolicyBridge, CedarPolicyEngine, CedarRiskLevel,
    PolicyValidationError, PolicyValidationResult, PolicyValidator, ToolAuthorizationRequest,
    ToolOperation,
};
pub use commit_strategy::{
    CommitMetadata, CommitRequest, CommitResult, CommitStrategy, CommitStrategyError,
};
pub use conflict_resolution::{
    ConflictDetectionResult, ConflictHunk, ConflictInfo, ConflictResolutionError, ConflictResolver,
    ConflictResolverConfig, FileOwner, ResolutionResult, ResolutionStrategy,
};
pub use credential_proxy::{
    AccessToken, Credential, CredentialProvider, CredentialProxy, CredentialProxyError,
    CredentialScope, EnvCredentialProvider,
};
pub use egress::{
    presets, Destination, EgressCheckResult, EgressDecision, EgressFilter, EgressFilterConfig,
    EgressRule, IpNetwork,
};
pub use executor::ToolExecutor;
pub use hybrid::{
    ExecutorInput, HybridConfig, HybridExecutor, LocalExecutor, RemoteExecutor, RiskClass,
    ToolExecutorTrait,
};
pub use os_sandbox::{
    detect_available_sandbox, detect_available_sandbox_sync, BubblewrapSandbox,
    FilesystemPermission, LandlockSandbox, NetworkPolicy, OsSandbox, OsSandboxConfig,
    PlatformSandbox, SandboxAvailability, SandboxType, SeatbeltSandbox,
};
pub use pr_creation::{
    EvidenceSummary, PrCreationError, PrCreator, PrCreatorConfig, PrLabel, PrMetadata, PrResult,
};
pub use secret_scanning::{
    install_precommit_hook, DetectedSecret, SecretScanResult, SecretScanner,
    SecretScannerConfig, SecretScannerError, SecretScannerType,
};
pub use registry::{ToolCategory, ToolRegistry, CategoryInfo, ToolRegistration};
pub use vault::{
    AwsCredentials, DatabaseCredentials, DynamicSecretResponse, DynamicSecretType, VaultClient,
    VaultClientConfig, VaultCredentialProvider, VaultDynamicSecret, VaultError,
};
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
        registry
            .register(tools::ReadFileTool::new(), registry::ToolCategory::File)
            .await;
        assert_eq!(registry.list().await.len(), 1);
    }
}
