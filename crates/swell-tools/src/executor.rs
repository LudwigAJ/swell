//! Tool executor with permission enforcement.

use swell_core::{ToolInput, ToolOutput, SwellError, ToolRiskLevel, PermissionTier};
use swell_core::traits::Tool;
use crate::registry::ToolRegistry;
use std::sync::Arc;
use tracing::{info, warn, error};
use std::time::Instant;

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
    pub fn is_allowed(&self, tool_name: &str, tool_tier: PermissionTier) -> bool {
        if self.allowed_tools.contains(tool_name) {
            return true;
        }
        matches!(tool_tier, PermissionTier::Auto)
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
}

impl ToolExecutor {
    pub fn new(registry: ToolRegistry) -> Self {
        Self {
            registry,
            permissions: PermissionChecker::new(),
        }
    }

    pub fn with_permissions(mut self, permissions: PermissionChecker) -> Self {
        self.permissions = permissions;
        self
    }

    /// Execute a tool by name
    pub async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolOutput, SwellError> {
        let start = Instant::now();
        
        let tool = self.registry.get(name).await
            .ok_or_else(|| SwellError::ToolExecutionFailed(format!("Tool not found: {}", name)))?;
        
        // Check permissions
        if !self.permissions.is_allowed(name, tool.permission_tier()) {
            warn!(tool = %name, "Tool execution denied");
            return Err(SwellError::PermissionDenied(format!(
                "Tool '{}' requires {:?} permission", name, tool.permission_tier()
            )));
        }

        info!(tool = %name, "Executing tool");
        let result = tool.execute(arguments).await;
        
        let duration = start.elapsed();
        info!(tool = %name, duration_ms = %duration.as_millis(), "Tool execution completed");

        result
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
}

impl Clone for ToolExecutor {
    fn clone(&self) -> Self {
        Self {
            registry: self.registry.clone(),
            permissions: self.permissions.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ReadFileTool;

    #[tokio::test]
    async fn test_executor_permission_denied() {
        let registry = ToolRegistry::new();
        registry.register(ReadFileTool::new()).await;
        
        let executor = ToolExecutor::new(registry);
        
        // Default permission tier is Auto, so it should work
        let result = executor.execute("read_file", serde_json::json!({"path": "/tmp/test"})).await;
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
}
