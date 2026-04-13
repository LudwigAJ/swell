# swell-tools AGENTS.md

## Purpose

`swell-tools` provides the tool implementations for the SWELL autonomous coding engine. It offers a central `ToolRegistry` for discovering and executing tools, plus concrete implementations for file I/O, git operations, shell execution, search, MCP client, and policy enforcement.

This crate handles:
- **Tool Registry** — Central registry for all tools with permission tiers (Builtin, Plugin, Runtime)
- **File Tools** — Read, write, edit files with safety checks
- **Git Tools** — Status, diff, commit, branch operations
- **Shell Execution** — Sandboxed command execution
- **Search Tools** — Grep, glob for codebase exploration
- **MCP Client** — Connect to external MCP servers for additional tools
- **Cedar Policy** — Formally verifiable authorization policy evaluation
- **Vault** — Secret management integration
- **Loop Detection** — Doom loop prevention

**Depends on:** `swell-core` (for `Tool` trait and `SwellError`)

## Public API

### Tool Registry (`registry.rs`)

```rust
pub struct ToolRegistry { /* ... */ }

impl ToolRegistry {
    pub fn new() -> Self;
    pub async fn register(&self, tool: Arc<dyn Tool>, category: ToolCategory) -> Result<(), ToolError>;
    pub async fn list(&self) -> Vec<ToolInfo>;
    pub async fn get(&self, name: &str) -> Option<Arc<dyn Tool>>;
    pub async fn unregister(&self, name: &str) -> Option<Arc<dyn Tool>>;
}

pub enum ToolCategory {
    File,
    Git,
    Shell,
    Search,
    Lsp,
    Web,
    Custom,
}
```

### Tool Executor (`executor.rs`)

```rust
pub struct ToolExecutor { /* ... */ }

impl ToolExecutor {
    pub fn new(registry: Arc<ToolRegistry>) -> Self;
    pub async fn execute(&self, name: &str, input: Value) -> Result<ToolOutput, SwellError>;
    pub async fn execute_with_policy(&self, name: &str, input: Value, policy: &CedarPolicyEngine) -> Result<ToolOutput, SwellError>;
}
```

### Cedar Policy (`cedar_policy.rs`)

```rust
pub struct CedarPolicyEngine { /* ... */ }
pub struct CedarPolicyBridge { /* ... */ }

pub enum CedarDecision { Allow, Deny }

pub enum CedarRiskLevel {
    Safe,
    LowRisk,
    MediumRisk,
    HighRisk,
    Critical,
}
```

### MCP Client (`mcp.rs`)

```rust
pub struct McpClient { /* ... */ }
pub struct McpManager { /* ... */ }

impl McpClient {
    pub async fn connect(config: &McpServerConfig) -> Result<Self, McpError>;
    pub async fn list_tools(&self) -> Result<Vec<McpToolInfo>, McpError>;
    pub async fn call_tool(&self, name: &str, args: Value) -> Result<Value, McpError>;
}
```

### Worktree Isolation (`worktree_isolation.rs`, `worktree_pool.rs`)

```rust
pub struct WorktreeIsolation { /* ... */ }
pub struct WorktreePool { /* ... */ }

impl WorktreePool {
    pub fn new(config: WorktreePoolConfig) -> Self;
    pub async fn acquire(&self) -> Result<WorktreeAllocation, WorktreePoolError>;
    pub async fn release(&self, allocation: WorktreeAllocation);
}
```

### Loop Detection (`loop_detection.rs`)

```rust
pub struct ToolLoopTracker { /* ... */ }

impl ToolLoopTracker {
    pub fn record(&mut self, tool_name: &str, success: bool);
    pub fn analyze(&self) -> LoopDetectionResult;
}
```

### Key Re-exports

```rust
pub use registry::{ToolRegistry, ToolCategory};
pub use executor::ToolExecutor;
pub use cedar_policy::{CedarPolicyEngine, CedarDecision, CedarRiskLevel};
pub use mcp::{McpClient, McpManager, McpToolInfo};
pub use worktree_pool::{WorktreePool, WorktreeAllocation, WorktreePoolConfig};
pub use loop_detection::{ToolLoopTracker, LoopDetectionResult};
pub use vault::{VaultClient, VaultClientConfig};
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                         swell-tools                                │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌───────────┐ │
│  │   Tools     │  │    Git      │  │   Shell     │  │  Search   │ │
│  │  file.rs    │  │   git.rs    │  │  shell.rs   │  │  search.rs│ │
│  └─────────────┘  └─────────────┘  └─────────────┘  └───────────┘ │
│                                                                     │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌───────────┐ │
│  │  Registry   │  │  Executor   │  │   Cedar     │  │    MCP    │ │
│  │ registry.rs │  │ executor.rs │  │cedar_policy │  │   mcp.rs  │ │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘  └─────┬─────┘ │
│         │                 │                 │              │       │
│         └─────────────────┼─────────────────┼──────────────┘       │
│                           │                 │                      │
│                    ┌─────▼─────────────────▼──────┐               │
│                    │        ToolExecutor            │               │
│                    │   (permission enforcement)     │               │
│                    └───────────────┬────────────────┘               │
│                                │                                   │
│                    ┌───────────▼───────────┐                       │
│                    │    CedarPolicyEngine  │                       │
│                    │  (authorization)      │                       │
│                    └──────────────────────┘                       │
│                                                                     │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐                  │
│  │   Vault    │  │   Loop      │  │  Worktree   │                  │
│  │  vault.rs  │  │ Detection   │  │   Pool      │                  │
│  └─────────────┘  └─────────────┘  └─────────────┘                  │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
                           │ used by
                           ▼
              ┌────────────────────────┐
              │   swell-orchestrator    │
              │   swell-validation      │
              └────────────────────────┘
```

**Key modules:**
- `registry.rs` — Tool registry with Builtin/Plugin/Runtime layers
- `executor.rs` — Tool execution with permission checks
- `cedar_policy.rs` — Cedar policy engine for authorization
- `mcp.rs` — MCP client for external tool servers
- `mcp_lsp.rs` — LSP bridge for code intelligence (rust-analyzer, etc.)
- `tools.rs` — Built-in tools (file I/O, git, shell, search)
- `vault.rs` — Secret management
- `loop_detection.rs` — Doom loop prevention
- `worktree_pool.rs` — Git worktree pool for agent isolation
- `commit_strategy.rs` — Atomic commits with metadata trailers
- `pr_creation.rs` — PR creation with evidence and labels

**Concurrency:** Uses `Arc<RwLock<T>>` for shared state. Tools must be `Send + Sync`.

## Testing

```bash
# Run tests for swell-tools
cargo test -p swell-tools -- --test-threads=4

# Run with logging
RUST_LOG=debug cargo test -p swell-tools

# Run specific test module
cargo test -p swell-tools -- registry --nocapture

# Run MCP tests
cargo test -p swell-tools -- mcp

# Run Cedar policy tests
cargo test -p swell-tools -- cedar

# Run LSP tests
cargo test -p swell-tools -- lsp_
```

**Test patterns:**
- Unit tests for each tool implementation
- Integration tests for registry operations
- MCP client tests with mock servers
- Policy evaluation tests
- Loop detection tests

**Mock patterns:**
```rust
#[tokio::test]
async fn test_registry_empty() {
    let registry = ToolRegistry::new();
    assert!(registry.list().await.is_empty());
}

#[tokio::test]
async fn test_registry_registration() {
    let registry = ToolRegistry::new();
    registry.register(tools::ReadFileTool::new(), registry::ToolCategory::File).await;
    assert_eq!(registry.list().await.len(), 1);
}
```

## Dependencies

```toml
# swell-tools/Cargo.toml
[dependencies]
swell-core = { path = "../swell-core" }
tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
anyhow.workspace = true
tracing.workspace = true
uuid.workspace = true
chrono.workspace = true
async-trait.workspace = true
futures.workspace = true
reqwest.workspace = true
glob.workspace = true
regex.workspace = true
tempfile.workspace = true
which.workspace = true
urlencoding = "2.1"
url = "2.5"

# Cedar policy engine for formally verifiable authorization
cedar-policy = "4"
cedar-policy-core = "4"

[dev-dependencies]
tokio-test.workspace = true
mockall.workspace = true
```
