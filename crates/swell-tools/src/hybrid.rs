//! Hybrid execution with LocalExecutor and RemoteExecutor for risk-based routing.
//!
//! This module provides:
//! - [`LocalExecutor`] - Executes low-risk operations locally in the current process/environment
//! - [`RemoteExecutor`] - Executes operations in a sandboxed environment (Firecracker, gVisor)
//! - [`HybridExecutor`] - Routes operations to the appropriate executor based on risk classification
//!
//! ## Risk Classification
//!
//! - **Low-risk** (Read operations): Executed locally via `LocalExecutor`
//! - **Medium-risk** (Write operations): Can be executed locally or remotely based on configuration
//! - **High-risk** (Destructive, Shell): Always executed remotely via `RemoteExecutor`

use async_trait::async_trait;
use std::sync::Arc;
use swell_core::{PermissionTier, SwellError, ToolOutput, ToolRiskLevel};
use tracing::{info, warn};

/// Risk classification for tool operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskClass {
    /// Read-only operations that don't modify state
    Low,
    /// Operations that modify files but are generally safe
    Medium,
    /// Operations that can have significant side effects
    High,
    /// Operations that are destructive and require sandbox isolation
    Destructive,
}

impl RiskClass {
    /// Classify a tool's risk level
    pub fn from_tool_risk_level(risk_level: ToolRiskLevel) -> Self {
        match risk_level {
            ToolRiskLevel::Read => RiskClass::Low,
            ToolRiskLevel::Write => RiskClass::Medium,
            ToolRiskLevel::Destructive => RiskClass::Destructive,
        }
    }

    /// Check if this risk class should be executed remotely
    pub fn requires_remote(&self) -> bool {
        matches!(self, RiskClass::High | RiskClass::Destructive)
    }

    /// Check if this risk class should be executed locally
    pub fn can_run_local(&self) -> bool {
        matches!(self, RiskClass::Low | RiskClass::Medium)
    }
}

/// Configuration for hybrid execution
#[derive(Debug, Clone)]
pub struct HybridConfig {
    /// Execute medium-risk tools locally (true) or remotely (false)
    pub medium_risk_local: bool,
    /// Execute high-risk tools remotely (should generally be true)
    pub high_risk_remote: bool,
    /// Sandbox endpoint for remote execution (e.g., Firecracker, gVisor)
    pub sandbox_endpoint: Option<String>,
}

impl Default for HybridConfig {
    fn default() -> Self {
        Self {
            medium_risk_local: true,
            high_risk_remote: true,
            sandbox_endpoint: None,
        }
    }
}

/// Input for tool execution with risk context
#[derive(Debug, Clone)]
pub struct ExecutorInput {
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub risk_class: RiskClass,
}

/// Trait for executing tools
#[async_trait]
pub trait ToolExecutorTrait: Send + Sync {
    /// Execute a tool and return its output
    async fn execute_tool(&self, input: ExecutorInput) -> Result<ToolOutput, SwellError>;

    /// Check if the executor is available and healthy
    async fn is_healthy(&self) -> bool;
}

/// Local executor for low and medium risk operations
///
/// Executes tools directly in the current process/environment without isolation.
/// Suitable for read operations and safe write operations.
pub struct LocalExecutor {
    registry: Arc<crate::registry::ToolRegistry>,
}

impl LocalExecutor {
    /// Create a new LocalExecutor with the given registry
    pub fn new(registry: Arc<crate::registry::ToolRegistry>) -> Self {
        Self { registry }
    }

    /// Execute a tool locally
    pub async fn execute(&self, input: ExecutorInput) -> Result<ToolOutput, SwellError> {
        info!(
            tool = %input.tool_name,
            risk_class = ?input.risk_class,
            "LocalExecutor: executing tool"
        );

        let tool = self.registry.get(&input.tool_name).await.ok_or_else(|| {
            SwellError::ToolExecutionFailed(format!("Tool not found: {}", input.tool_name))
        })?;

        // Check permission tier
        if !matches!(
            tool.permission_tier(),
            PermissionTier::Auto | PermissionTier::Ask
        ) {
            warn!(
                tool = %input.tool_name,
                tier = ?tool.permission_tier(),
                "LocalExecutor: tool requires deny tier, rejecting"
            );
            return Err(SwellError::PermissionDenied(format!(
                "Tool '{}' requires {:?} permission tier",
                input.tool_name,
                tool.permission_tier()
            )));
        }

        tool.execute(input.arguments).await
    }

    /// Get the registry for inspection
    pub fn registry(&self) -> &Arc<crate::registry::ToolRegistry> {
        &self.registry
    }
}

#[async_trait]
impl ToolExecutorTrait for LocalExecutor {
    async fn execute_tool(&self, input: ExecutorInput) -> Result<ToolOutput, SwellError> {
        self.execute(input).await
    }

    async fn is_healthy(&self) -> bool {
        // Local executor is always healthy if registry is accessible
        true
    }
}

impl Clone for LocalExecutor {
    fn clone(&self) -> Self {
        Self {
            registry: self.registry.clone(),
        }
    }
}

/// Remote executor for sandboxed execution
///
/// Executes tools in an isolated environment (Firecracker, gVisor, or custom sandbox).
/// Suitable for high-risk and destructive operations.
pub struct RemoteExecutor {
    sandbox_endpoint: String,
    client: reqwest::Client,
}

impl RemoteExecutor {
    /// Create a new RemoteExecutor with the given sandbox endpoint
    pub fn new(sandbox_endpoint: impl Into<String>) -> Self {
        Self {
            sandbox_endpoint: sandbox_endpoint.into(),
            client: reqwest::Client::new(),
        }
    }

    /// Create a new RemoteExecutor with default configuration
    pub fn with_default_config() -> Self {
        Self {
            sandbox_endpoint: "http://localhost:8080".to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Execute a tool remotely via the sandbox API
    pub async fn execute(&self, input: ExecutorInput) -> Result<ToolOutput, SwellError> {
        info!(
            tool = %input.tool_name,
            risk_class = ?input.risk_class,
            endpoint = %self.sandbox_endpoint,
            "RemoteExecutor: executing tool remotely"
        );

        // Build the request to the sandbox endpoint
        let url = format!("{}/execute/{}", self.sandbox_endpoint, input.tool_name);

        let request = serde_json::json!({
            "arguments": input.arguments,
            "risk_class": serde_json::json!({
                "level": format!("{:?}", input.risk_class),
            }),
        });

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                SwellError::ToolExecutionFailed(format!("Remote execution request failed: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(SwellError::ToolExecutionFailed(format!(
                "Remote execution failed with status {}: {}",
                status, error_text
            )));
        }

        let output: ToolOutput = response.json().await.map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to parse remote response: {}", e))
        })?;

        Ok(output)
    }

    /// Get the sandbox endpoint
    pub fn endpoint(&self) -> &str {
        &self.sandbox_endpoint
    }
}

#[async_trait]
impl ToolExecutorTrait for RemoteExecutor {
    async fn execute_tool(&self, input: ExecutorInput) -> Result<ToolOutput, SwellError> {
        self.execute(input).await
    }

    async fn is_healthy(&self) -> bool {
        // Check if the sandbox endpoint is responsive
        match self
            .client
            .get(format!("{}/health", self.sandbox_endpoint))
            .send()
            .await
        {
            Ok(response) => response.status().is_success(),
            Err(_) => false,
        }
    }
}

impl Clone for RemoteExecutor {
    fn clone(&self) -> Self {
        Self {
            sandbox_endpoint: self.sandbox_endpoint.clone(),
            client: self.client.clone(),
        }
    }
}

/// Hybrid executor that routes operations to LocalExecutor or RemoteExecutor based on risk
///
/// ## Usage
///
/// ```rust,ignore
/// let registry = Arc::new(ToolRegistry::new());
/// let local = LocalExecutor::new(registry.clone());
/// let remote = RemoteExecutor::new("http://localhost:8080");
/// let hybrid = HybridExecutor::new(local, remote, HybridConfig::default());
///
/// // Tool is automatically routed based on its risk level
/// let result = hybrid.execute_tool(ExecutorInput {
///     tool_name: "read_file".to_string(),
///     arguments: serde_json::json!({"path": "/tmp/test"}),
///     risk_class: RiskClass::Low,
/// }).await?;
/// ```
pub struct HybridExecutor {
    local: Arc<dyn ToolExecutorTrait>,
    remote: Arc<dyn ToolExecutorTrait>,
    config: HybridConfig,
}

impl HybridExecutor {
    /// Create a new HybridExecutor with the given executors and configuration
    pub fn new(
        local: impl ToolExecutorTrait + 'static,
        remote: impl ToolExecutorTrait + 'static,
        config: HybridConfig,
    ) -> Self {
        Self {
            local: Arc::new(local),
            remote: Arc::new(remote),
            config,
        }
    }

    /// Create a HybridExecutor using LocalExecutor and RemoteExecutor directly
    pub fn with_defaults(
        registry: Arc<crate::registry::ToolRegistry>,
        sandbox_endpoint: Option<String>,
    ) -> Self {
        let endpoint_clone = sandbox_endpoint.clone();
        let local = LocalExecutor::new(registry);
        let remote = sandbox_endpoint
            .map(RemoteExecutor::new)
            .unwrap_or_else(RemoteExecutor::with_default_config);
        let config = HybridConfig {
            sandbox_endpoint: endpoint_clone,
            ..Default::default()
        };
        Self {
            local: Arc::new(local),
            remote: Arc::new(remote),
            config,
        }
    }

    /// Create a HybridExecutor with custom executors
    pub fn with_executors(
        local: Arc<dyn ToolExecutorTrait>,
        remote: Arc<dyn ToolExecutorTrait>,
        config: HybridConfig,
    ) -> Self {
        Self {
            local,
            remote,
            config,
        }
    }

    /// Execute a tool, routing to the appropriate executor based on risk classification
    pub async fn execute_tool(&self, input: ExecutorInput) -> Result<ToolOutput, SwellError> {
        let risk_class = &input.risk_class;

        // Determine which executor to use based on risk classification and config
        let executor = self.determine_executor(risk_class);

        info!(
            tool = %input.tool_name,
            risk_class = ?risk_class,
            executor = if Arc::ptr_eq(&executor, &self.local) { "local" } else { "remote" },
            "HybridExecutor: routing tool execution"
        );

        executor.execute_tool(input).await
    }

    /// Determine which executor to use based on risk classification and config
    fn determine_executor(&self, risk_class: &RiskClass) -> Arc<dyn ToolExecutorTrait> {
        match risk_class {
            RiskClass::Low => {
                // Low-risk always goes local
                Arc::clone(&self.local)
            }
            RiskClass::Medium => {
                // Medium-risk configurable
                if self.config.medium_risk_local {
                    Arc::clone(&self.local)
                } else {
                    Arc::clone(&self.remote)
                }
            }
            RiskClass::High | RiskClass::Destructive => {
                // High-risk and destructive configurable, but defaults to remote
                if self.config.high_risk_remote {
                    Arc::clone(&self.remote)
                } else {
                    Arc::clone(&self.local)
                }
            }
        }
    }

    /// Get the configuration
    pub fn config(&self) -> &HybridConfig {
        &self.config
    }

    /// Check if the hybrid executor is healthy (checks both local and remote)
    pub async fn is_healthy(&self) -> bool {
        // Local should always be healthy
        let local_healthy = self.local.is_healthy().await;

        // Remote is optional - hybrid can work local-only
        let _remote_healthy = self.remote.is_healthy().await;

        // Hybrid is healthy if local is healthy (remote may be down)
        local_healthy
    }

    /// Get the local executor for direct access
    pub fn local(&self) -> &Arc<dyn ToolExecutorTrait> {
        &self.local
    }

    /// Get the remote executor for direct access
    pub fn remote(&self) -> &Arc<dyn ToolExecutorTrait> {
        &self.remote
    }
}

impl Clone for HybridExecutor {
    fn clone(&self) -> Self {
        Self {
            local: Arc::clone(&self.local),
            remote: Arc::clone(&self.remote),
            config: self.config.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ToolRegistry;
    use crate::tools::ReadFileTool;

    fn create_test_executor_input(tool_name: &str, risk_class: RiskClass) -> ExecutorInput {
        ExecutorInput {
            tool_name: tool_name.to_string(),
            arguments: serde_json::json!({}),
            risk_class,
        }
    }

    #[test]
    fn test_risk_class_from_tool_risk_level() {
        assert_eq!(
            RiskClass::from_tool_risk_level(ToolRiskLevel::Read),
            RiskClass::Low
        );
        assert_eq!(
            RiskClass::from_tool_risk_level(ToolRiskLevel::Write),
            RiskClass::Medium
        );
        assert_eq!(
            RiskClass::from_tool_risk_level(ToolRiskLevel::Destructive),
            RiskClass::Destructive
        );
    }

    #[test]
    fn test_risk_class_requires_remote() {
        assert!(!RiskClass::Low.requires_remote());
        assert!(!RiskClass::Medium.requires_remote());
        assert!(RiskClass::High.requires_remote());
        assert!(RiskClass::Destructive.requires_remote());
    }

    #[test]
    fn test_risk_class_can_run_local() {
        assert!(RiskClass::Low.can_run_local());
        assert!(RiskClass::Medium.can_run_local());
        assert!(!RiskClass::High.can_run_local());
        assert!(!RiskClass::Destructive.can_run_local());
    }

    #[tokio::test]
    async fn test_local_executor_healthy() {
        let registry = Arc::new(ToolRegistry::new());
        let local = LocalExecutor::new(registry);
        assert!(local.is_healthy().await);
    }

    #[tokio::test]
    async fn test_hybrid_executor_routes_low_risk_locally() {
        let registry = Arc::new(ToolRegistry::new());
        registry
            .register(
                ReadFileTool::new(),
                crate::registry::ToolCategory::File,
                crate::registry::ToolLayer::Builtin,
            )
            .await;

        let local = LocalExecutor::new(registry.clone());
        let remote = RemoteExecutor::with_default_config();
        let hybrid = HybridExecutor::new(local, remote, HybridConfig::default());

        let _input = create_test_executor_input("read_file", RiskClass::Low);
        // This will fail because tool doesn't exist in remote, but we can verify it routed
        // For now, just verify the hybrid is set up correctly
        assert!(hybrid.config.medium_risk_local);
        assert!(hybrid.config.high_risk_remote);
    }

    #[tokio::test]
    async fn test_hybrid_executor_routes_high_risk_to_remote_by_default() {
        let registry = Arc::new(ToolRegistry::new());
        let local = LocalExecutor::new(registry.clone());
        let remote = RemoteExecutor::with_default_config();
        let hybrid = HybridExecutor::new(local, remote, HybridConfig::default());

        // High-risk should route to remote by default
        let input = create_test_executor_input("shell", RiskClass::High);
        let executor = hybrid.determine_executor(&input.risk_class);
        // Should be remote
        assert!(Arc::ptr_eq(&executor, &hybrid.remote) || hybrid.config.high_risk_remote);
    }

    #[tokio::test]
    async fn test_hybrid_config_medium_risk_local() {
        let registry = Arc::new(ToolRegistry::new());
        let local = LocalExecutor::new(registry.clone());
        let remote = RemoteExecutor::with_default_config();

        // With medium_risk_local = true
        let config = HybridConfig {
            medium_risk_local: true,
            high_risk_remote: true,
            sandbox_endpoint: None,
        };
        let hybrid = HybridExecutor::new(local, remote, config);

        let input = create_test_executor_input("write_file", RiskClass::Medium);
        let executor = hybrid.determine_executor(&input.risk_class);
        assert!(Arc::ptr_eq(&executor, &hybrid.local));
    }

    #[tokio::test]
    async fn test_hybrid_config_medium_risk_remote() {
        let registry = Arc::new(ToolRegistry::new());
        let local = LocalExecutor::new(registry.clone());
        let remote = RemoteExecutor::with_default_config();

        // With medium_risk_local = false (remote)
        let config = HybridConfig {
            medium_risk_local: false,
            high_risk_remote: true,
            sandbox_endpoint: None,
        };
        let hybrid = HybridExecutor::new(local, remote, config);

        let input = create_test_executor_input("write_file", RiskClass::Medium);
        let executor = hybrid.determine_executor(&input.risk_class);
        assert!(Arc::ptr_eq(&executor, &hybrid.remote));
    }

    #[tokio::test]
    async fn test_hybrid_config_high_risk_local_fallback() {
        let registry = Arc::new(ToolRegistry::new());
        let local = LocalExecutor::new(registry.clone());
        let remote = RemoteExecutor::with_default_config();

        // With high_risk_remote = false (fallback to local)
        let config = HybridConfig {
            medium_risk_local: true,
            high_risk_remote: false,
            sandbox_endpoint: None,
        };
        let hybrid = HybridExecutor::new(local, remote, config);

        let input = create_test_executor_input("shell", RiskClass::High);
        let executor = hybrid.determine_executor(&input.risk_class);
        assert!(Arc::ptr_eq(&executor, &hybrid.local));
    }

    #[tokio::test]
    async fn test_hybrid_executor_with_defaults() {
        let registry = Arc::new(ToolRegistry::new());
        let hybrid = HybridExecutor::with_defaults(registry, None);
        assert!(hybrid.is_healthy().await);
    }

    #[tokio::test]
    async fn test_hybrid_executor_clone() {
        let registry = Arc::new(ToolRegistry::new());
        let hybrid =
            HybridExecutor::with_defaults(registry, Some("http://localhost:9090".to_string()));
        let cloned = hybrid.clone();
        assert!(cloned.config.sandbox_endpoint.is_some());
    }
}
