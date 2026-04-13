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

pub mod auto_masking;
pub mod branch_strategy;
pub mod cedar_policy;
pub mod commit_strategy;
pub mod conflict_resolution;
pub mod credential_proxy;
pub mod egress;
pub mod executor;
pub mod hooks;
pub mod hybrid;
pub mod killswitch;
pub mod loop_detection;
pub mod mcp;
pub mod mcp_config;
pub mod mcp_lsp;
pub mod opa_policy;
pub mod os_sandbox;
pub mod post_tool_hooks;
pub mod pr_creation;
pub mod registry;
pub mod resource_limits;
pub mod secret_scanning;
pub mod self_healing_ci;
pub mod tools;
pub mod vault;
pub mod web_search;
pub mod worktree_isolation;
pub mod worktree_pool;

pub use auto_masking::{
    AutoMasker, MaskSecrets, MaskingConfig, MaskingResult, MaskingStats, SecretPattern,
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
pub use killswitch::ToolKillSwitch;
pub use loop_detection::{
    create_tool_loop_tracker, create_tool_loop_tracker_with_config, LoopDetectionConfig,
    LoopDetectionResult, LoopPattern, LoopPatternType, SharedToolLoopTracker, ToolExecution,
    ToolLoopTracker,
};
pub use mcp::{
    McpClient, McpManager, McpToolInfo, McpToolWrapper,
};
pub use mcp_config::{
    McpConfigManager, McpReconnectConfig, McpServerConfig, McpServerHealth, McpServerState,
    McpServersConfig,
};
pub use mcp_lsp::{
    LspBridgeManager, LspDefinitionTool, LspDiagnostic, LspDiagnosticsTool,
    LspHover, LspHoverTool, LspLanguage, LspLocation, LspPosition, LspRange, LspReferencesTool,
    LspRenameResult, LspRenameTool, LspSymbol, LspTextEdit, LspWorkspaceDiagnostics,
};
pub use os_sandbox::{
    detect_available_sandbox, detect_available_sandbox_sync, BubblewrapSandbox,
    FilesystemPermission, LandlockSandbox, NetworkPolicy, OsSandbox, OsSandboxConfig,
    PlatformSandbox, SandboxAvailability, SandboxType, SeatbeltSandbox,
};
pub use post_tool_hooks::{
    tool_names, FormatHook, HookConfig, HookResult, HookTrigger, LintHook, PostToolHook,
    PostToolHookManager,
};
pub use pr_creation::{
    EvidenceSummary, PrCreationError, PrCreator, PrCreatorConfig, PrLabel, PrMetadata, PrResult,
};
pub use registry::{CategoryInfo, ToolCategory, ToolRegistration, ToolRegistry};
pub use resource_limits::{
    LimitCheckResult, LimitState, ResourceLimitError, ResourceLimitResult, SessionLimits,
    SessionResourceTracker,
};
pub use secret_scanning::{
    install_precommit_hook, DetectedSecret, SecretScanResult, SecretScanner, SecretScannerConfig,
    SecretScannerError, SecretScannerType,
};
pub use self_healing_ci::{
    CiFailure, CiFailureAnalysis, CiFix, CiHealingResult, CiHealingTool, CiSeverity, CodeChange,
    FailureCategory, FixType, RootCause,
};
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
