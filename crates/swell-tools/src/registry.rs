//! Tool registry for managing available tools.

use std::collections::HashMap;
use std::sync::Arc;
use swell_core::traits::Tool;
use swell_core::{PermissionTier, ToolRiskLevel};
use tokio::sync::RwLock;

/// A registered tool with metadata
#[derive(Clone)]
pub struct ToolRegistration {
    pub name: String,
    pub description: String,
    pub risk_level: ToolRiskLevel,
    pub permission_tier: PermissionTier,
    pub tool: Arc<dyn Tool>,
}

/// Central registry for all available tools
pub struct ToolRegistry {
    tools: Arc<RwLock<HashMap<String, ToolRegistration>>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a tool
    pub async fn register<T: Tool + 'static>(&self, tool: T) {
        let registration = ToolRegistration {
            name: tool.name().to_string(),
            description: tool.description(),
            risk_level: tool.risk_level(),
            permission_tier: tool.permission_tier(),
            tool: Arc::new(tool),
        };

        let mut tools = self.tools.write().await;
        tools.insert(registration.name.clone(), registration);
    }

    /// Get a tool by name
    pub async fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        let tools = self.tools.read().await;
        tools.get(name).map(|r| r.tool.clone())
    }

    /// List all registered tool names
    pub async fn list_names(&self) -> Vec<String> {
        let tools = self.tools.read().await;
        tools.keys().cloned().collect()
    }

    /// List all tool registrations
    pub async fn list(&self) -> Vec<ToolRegistration> {
        let tools = self.tools.read().await;
        tools.values().cloned().collect()
    }

    /// Check if a tool is registered
    pub async fn contains(&self, name: &str) -> bool {
        let tools = self.tools.read().await;
        tools.contains_key(name)
    }

    /// Remove a tool
    pub async fn unregister(&self, name: &str) -> bool {
        let mut tools = self.tools.write().await;
        tools.remove(name).is_some()
    }

    /// Get tools filtered by risk level
    pub async fn by_risk_level(&self, level: ToolRiskLevel) -> Vec<ToolRegistration> {
        let tools = self.tools.read().await;
        tools
            .values()
            .filter(|r| r.risk_level == level)
            .cloned()
            .collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ToolRegistry {
    fn clone(&self) -> Self {
        Self {
            tools: self.tools.clone(),
        }
    }
}
