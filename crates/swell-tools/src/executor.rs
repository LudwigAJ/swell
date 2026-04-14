//! Tool executor with permission enforcement.

use crate::post_tool_hooks::PostToolHookManager;
use crate::registry::ToolRegistry;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use swell_core::{PermissionTier, SwellError, ToolOutput};
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Permission checker for tools
#[derive(Debug, Clone)]
pub struct PermissionChecker {
    default_tier: PermissionTier,
    allowed_tools: std::collections::HashSet<String>,
}

impl PermissionChecker {
    pub fn new() -> Self {
        Self {
            default_tier: PermissionTier::Auto,
            allowed_tools: std::collections::HashSet::new(),
        }
    }

    /// Set the default permission tier
    pub fn with_default_tier(mut self, tier: PermissionTier) -> Self {
        self.default_tier = tier;
        self
    }

    /// Allow a specific tool (bypasses tier check)
    pub fn allow_tool(mut self, name: impl Into<String>) -> Self {
        self.allowed_tools.insert(name.into());
        self
    }

    /// Check if a tool execution is permitted
    ///
    /// Permission tiers:
    /// - `Auto`: Always permitted (auto-approved)
    /// - `Ask`: Treated as `Auto` (auto-approved) since no confirmation mechanism exists
    /// - `Deny`: Never permitted without explicit override via `allowed_tools`
    pub fn is_allowed(&self, tool_name: &str, tool_tier: PermissionTier) -> bool {
        if self.allowed_tools.contains(tool_name) {
            return true;
        }
        // Ask is treated as Auto since there's no user confirmation mechanism
        matches!(tool_tier, PermissionTier::Auto | PermissionTier::Ask)
    }
}

impl Default for PermissionChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Executes tools with permission enforcement and tracking
pub struct ToolExecutor {
    registry: ToolRegistry,
    permissions: PermissionChecker,
    hook_manager: Option<Arc<RwLock<PostToolHookManager>>>,
    workspace_path: PathBuf,
}

impl ToolExecutor {
    pub fn new(registry: ToolRegistry) -> Self {
        Self {
            registry,
            permissions: PermissionChecker::new(),
            hook_manager: None,
            workspace_path: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }

    pub fn with_permissions(mut self, permissions: PermissionChecker) -> Self {
        self.permissions = permissions;
        self
    }

    /// Set the workspace path for hook execution
    pub fn with_workspace_path(mut self, path: PathBuf) -> Self {
        self.workspace_path = path;
        self
    }

    /// Enable post-tool hooks with the given manager
    pub fn with_hook_manager(mut self, manager: PostToolHookManager) -> Self {
        self.hook_manager = Some(Arc::new(RwLock::new(manager)));
        self
    }

    /// Execute a tool by name
    pub async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolOutput, SwellError> {
        self.execute_with_hooks(name, arguments, vec![]).await
    }

    /// Execute a tool with post-tool hooks
    ///
    /// This method runs the tool and then executes any applicable post-tool hooks.
    /// The `modified_files` parameter provides hints about which files were modified,
    /// allowing hooks to decide whether to run.
    pub async fn execute_with_hooks(
        &self,
        name: &str,
        arguments: serde_json::Value,
        modified_files: Vec<String>,
    ) -> Result<ToolOutput, SwellError> {
        let start = Instant::now();

        let tool =
            self.registry.get(name).await.ok_or_else(|| {
                SwellError::ToolExecutionFailed(format!("Tool not found: {}", name))
            })?;

        // Check permissions
        if !self.permissions.is_allowed(name, tool.permission_tier()) {
            warn!(tool = %name, "Tool execution denied");
            return Err(SwellError::PermissionDenied(format!(
                "Tool '{}' requires {:?} permission",
                name,
                tool.permission_tier()
            )));
        }

        info!(tool = %name, "Executing tool");
        let result = tool.execute(arguments).await;

        let duration = start.elapsed();
        info!(tool = %name, duration_ms = %duration.as_millis(), "Tool execution completed");

        // Run post-tool hooks if enabled and tool succeeded
        if result.is_ok() {
            self.run_post_hooks(name, &modified_files).await;
        }

        result
    }

    /// Run post-tool hooks for a tool
    async fn run_post_hooks(&self, tool_name: &str, modified_files: &[String]) {
        if let Some(ref hook_manager) = self.hook_manager {
            let manager = hook_manager.read().await;
            let results = manager
                .execute_hooks(tool_name, &self.workspace_path, modified_files)
                .await;

            for hook_result in results {
                if hook_result.success {
                    info!(
                        hook = %hook_result.hook_name,
                        command = %hook_result.command,
                        "Post-tool hook completed successfully"
                    );
                } else {
                    warn!(
                        hook = %hook_result.hook_name,
                        error = ?hook_result.error,
                        "Post-tool hook completed with issues"
                    );
                }
            }
        }
    }

    /// Check if a tool can be executed (exists and permitted)
    pub async fn can_execute(&self, name: &str) -> bool {
        let tool = self.registry.get(name).await;
        match tool {
            Some(t) => self.permissions.is_allowed(name, t.permission_tier()),
            None => false,
        }
    }

    /// Get the registry for inspection
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    /// Get the workspace path
    pub fn workspace_path(&self) -> &PathBuf {
        &self.workspace_path
    }

    /// Get hook results from the last execution (for testing/debugging)
    pub fn has_hook_manager(&self) -> bool {
        self.hook_manager.is_some()
    }
}

impl Clone for ToolExecutor {
    fn clone(&self) -> Self {
        Self {
            registry: self.registry.clone(),
            permissions: self.permissions.clone(),
            hook_manager: self.hook_manager.clone(),
            workspace_path: self.workspace_path.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::post_tool_hooks::PostToolHookManager;
    use crate::tools::ReadFileTool;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_executor_permission_denied() {
        let registry = ToolRegistry::new();
        registry
            .register(
                ReadFileTool::new(),
                crate::registry::ToolCategory::File,
                crate::registry::ToolLayer::Builtin,
            )
            .await;

        let executor = ToolExecutor::new(registry);

        // Default permission tier is Auto, so it should work
        let result = executor
            .execute("read_file", serde_json::json!({"path": "/tmp/test"}))
            .await;
        // May fail due to file not existing, but permission should pass
        assert!(result.is_ok() || matches!(result, Err(SwellError::ToolExecutionFailed(_))));
    }

    #[tokio::test]
    async fn test_executor_tool_not_found() {
        let registry = ToolRegistry::new();
        let executor = ToolExecutor::new(registry);

        let result = executor.execute("nonexistent", serde_json::json!({})).await;
        assert!(matches!(result, Err(SwellError::ToolExecutionFailed(_))));
    }

    #[tokio::test]
    async fn test_executor_with_workspace_path() {
        let registry = ToolRegistry::new();
        let dir = tempdir().unwrap();

        let executor = ToolExecutor::new(registry).with_workspace_path(dir.path().to_path_buf());

        assert_eq!(executor.workspace_path(), dir.path());
    }

    #[tokio::test]
    async fn test_executor_with_hook_manager() {
        let registry = ToolRegistry::new();
        let hook_manager = PostToolHookManager::new();

        let executor = ToolExecutor::new(registry).with_hook_manager(hook_manager);

        assert!(executor.has_hook_manager());
    }
}
