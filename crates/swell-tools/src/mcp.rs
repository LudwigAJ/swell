//! MCP (Model Context Protocol) client for external tool servers.

use swell_core::{ToolOutput, SwellError, ToolRiskLevel, PermissionTier};
use swell_core::traits::Tool;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::warn;

/// MCP client for connecting to MCP servers
#[derive(Debug, Clone)]
pub struct McpClient {
    server_url: String,
    transport: Arc<RwLock<Option<McpTransport>>>,
}

#[derive(Debug)]
struct McpTransport {
    read: tokio::io::BufReader<tokio::net::UnixStream>,
    write: tokio::io::BufWriter<tokio::net::UnixStream>,
}

#[derive(Debug, Serialize, Deserialize)]
struct McpMessage {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<McpError>,
}

#[derive(Debug, Serialize, Deserialize)]
struct McpError {
    code: i32,
    message: String,
}

impl McpClient {
    pub fn new(server_url: impl Into<String>) -> Self {
        Self {
            server_url: server_url.into(),
            transport: Arc::new(RwLock::new(None)),
        }
    }

    /// Connect to the MCP server
    pub async fn connect(&self) -> Result<(), SwellError> {
        // For now, this is a placeholder
        // Full implementation would establish transport connection
        warn!(url = %self.server_url, "MCP connect not fully implemented");
        Ok(())
    }

    /// Disconnect from the MCP server
    pub async fn disconnect(&self) {
        let mut transport = self.transport.write().await;
        *transport = None;
    }

    /// List available tools from the MCP server
    pub async fn list_tools(&self) -> Result<Vec<McpToolInfo>, SwellError> {
        // Placeholder - full implementation would call the MCP server
        Ok(vec![])
    }

    /// Call an MCP tool
    pub async fn call_tool(
        &self,
        name: &str,
        _arguments: serde_json::Value,
    ) -> Result<ToolOutput, SwellError> {
        // Placeholder - full implementation would send JSON-RPC request
        Err(SwellError::ToolExecutionFailed(format!(
            "MCP tool call not implemented for {}", name
        )))
    }
}

/// Information about an MCP tool
#[derive(Debug, Clone, Deserialize)]
pub struct McpToolInfo {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Wrapper tool for MCP tools
struct McpToolWrapper {
    name: String,
    description: String,
    input_schema: serde_json::Value,
    client: McpClient,
}

#[async_trait]
impl Tool for McpToolWrapper {
    fn name(&self) -> &str { &self.name }
    fn description(&self) -> String { self.description.clone() }
    fn risk_level(&self) -> ToolRiskLevel { ToolRiskLevel::Read } // MCP tools default to Read
    fn permission_tier(&self) -> PermissionTier { PermissionTier::Ask }
    fn input_schema(&self) -> serde_json::Value { self.input_schema.clone() }

    async fn execute(&self, arguments: serde_json::Value) -> Result<ToolOutput, SwellError> {
        self.client.call_tool(&self.name, arguments).await
    }
}

/// Manager for MCP server connections
#[derive(Debug, Clone)]
pub struct McpManager {
    clients: Arc<RwLock<HashMap<String, McpClient>>>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add an MCP server connection
    pub async fn add_server(&self, name: String, url: String) -> Result<(), SwellError> {
        let client = McpClient::new(url);
        client.connect().await?;
        
        let mut clients = self.clients.write().await;
        clients.insert(name, client);
        
        Ok(())
    }

    /// Remove an MCP server connection
    pub async fn remove_server(&self, name: &str) -> bool {
        let mut clients = self.clients.write().await;
        if let Some(client) = clients.remove(name) {
            client.disconnect().await;
            true
        } else {
            false
        }
    }

    /// Get a client by name
    pub async fn get_client(&self, name: &str) -> Option<McpClient> {
        let clients = self.clients.read().await;
        clients.get(name).cloned()
    }

    /// List all connected servers
    pub async fn list_servers(&self) -> Vec<String> {
        let clients = self.clients.read().await;
        clients.keys().cloned().collect()
    }
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mcp_manager_empty() {
        let manager = McpManager::new();
        let servers = manager.list_servers().await;
        assert!(servers.is_empty());
    }

    #[tokio::test]
    async fn test_mcp_client_creation() {
        let client = McpClient::new("http://localhost:8080");
        assert_eq!(client.server_url, "http://localhost:8080");
    }
}
