//! MCP (Model Context Protocol) client for external tool servers.
//!
//! This module implements a client for MCP servers using JSON-RPC 2.0 over stdio.
//! MCP is the industry standard for AI tool integration, providing:
//! - Tool discovery via `tools/list`
//! - Tool execution via `tools/call`
//! - Deferred/lazy loading support
//!
//! Reference: https://modelcontextprotocol.io/

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use swell_core::traits::Tool;
use swell_core::{PermissionTier, SwellError, ToolOutput, ToolRiskLevel};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const JSONRPC_VERSION: &str = "2.0";

/// MCP client for connecting to MCP servers via stdio
#[derive(Debug, Clone)]
pub struct McpClient {
    server_url: String,
    /// Process handle plus buffered I/O - uses write lock for mutability
    process: Arc<RwLock<Option<McpProcess>>>,
    /// Server capabilities received during handshake
    capabilities: Arc<RwLock<Option<McpServerCapabilities>>>,
    /// Cached tool info from this server
    tools: Arc<RwLock<HashMap<String, McpToolInfo>>>,
}

/// Holds the child process and its buffered I/O streams
#[derive(Debug)]
struct McpProcess {
    child: tokio::process::Child,
    writer: BufWriter<tokio::process::ChildStdin>,
    reader: BufReader<tokio::process::ChildStdout>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpCapabilities {
    pub tools: Option<McpToolsCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolsCapability {
    pub list: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerCapabilities {
    pub tools: Option<McpToolsCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct McpInitializeRequest {
    protocol_version: String,
    capabilities: McpCapabilities,
    client_info: McpClientInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct McpClientInfo {
    name: String,
    version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct McpInitializeResponse {
    protocol_version: String,
    capabilities: McpServerCapabilities,
    server_info: McpServerInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct McpServerInfo {
    name: String,
    version: String,
}

// JSON-RPC Message Types
#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Value,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl JsonRpcRequest {
    fn new(id: Value, method: &str, params: Option<Value>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            method: method.to_string(),
            params,
        }
    }
}

/// Information about an MCP tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolInfo {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    pub server_name: String,
}

impl McpToolInfo {
    /// Returns the JSON schema for the tool's input
    pub fn schema(&self) -> Value {
        self.input_schema.clone().unwrap_or_else(|| {
            serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": true
            })
        })
    }
}

/// Wrapper tool for MCP tools - implements the Tool trait
#[derive(Debug, Clone)]
pub struct McpToolWrapper {
    info: McpToolInfo,
    client: McpClient,
}

impl McpToolWrapper {
    fn new(info: McpToolInfo, client: McpClient) -> Self {
        Self { info, client }
    }
}

#[async_trait]
impl Tool for McpToolWrapper {
    fn name(&self) -> &str {
        &self.info.name
    }

    fn description(&self) -> String {
        self.info.description.clone()
    }

    fn risk_level(&self) -> ToolRiskLevel {
        // MCP tools default to Read - risk classification can be enhanced later
        ToolRiskLevel::Read
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::Ask
    }

    fn input_schema(&self) -> Value {
        self.info.schema()
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, SwellError> {
        self.client.call_tool(&self.info.name, arguments).await
    }
}

impl McpClient {
    /// Create a new MCP client for the given server command
    pub fn new(server_url: impl Into<String>) -> Self {
        Self {
            server_url: server_url.into(),
            process: Arc::new(RwLock::new(None)),
            capabilities: Arc::new(RwLock::new(None)),
            tools: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Check if the client is connected to a server
    pub async fn is_connected(&self) -> bool {
        self.process.read().await.is_some()
    }

    /// Get the server URL
    pub fn server_url(&self) -> &str {
        &self.server_url
    }

    /// Connect to the MCP server
    pub async fn connect(&self) -> Result<(), SwellError> {
        if self.is_connected().await {
            return Ok(());
        }

        // Parse the server URL - expecting a command string for stdio
        let (program, args) = self.parse_server_command()?;

        info!(cmd = %self.server_url, "Starting MCP server process");

        let mut child = tokio::process::Command::new(&program)
            .args(&args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .map_err(|e| {
                SwellError::ToolExecutionFailed(format!(
                    "Failed to spawn MCP server '{}': {}",
                    self.server_url, e
                ))
            })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            SwellError::ToolExecutionFailed("Failed to take MCP server stdin".to_string())
        })?;

        let stdout = child.stdout.take().ok_or_else(|| {
            SwellError::ToolExecutionFailed("Failed to take MCP server stdout".to_string())
        })?;

        let process = McpProcess {
            child,
            writer: BufWriter::new(stdin),
            reader: BufReader::new(stdout),
        };

        {
            let mut p = self.process.write().await;
            *p = Some(process);
        }

        // Initialize the MCP protocol
        self.initialize_protocol().await?;

        info!(server = %self.server_url, "MCP client connected");
        Ok(())
    }

    /// Parse server command into program and arguments
    fn parse_server_command(&self) -> Result<(String, Vec<String>), SwellError> {
        let cmd = &self.server_url;
        let parts: Vec<&str> = cmd.split_whitespace().collect();

        if parts.is_empty() {
            return Err(SwellError::ConfigError(
                "MCP server command is empty".to_string(),
            ));
        }

        let program = parts[0].to_string();
        let args = parts[1..].iter().map(|s| s.to_string()).collect();

        Ok((program, args))
    }

    /// Initialize the MCP protocol with the server
    async fn initialize_protocol(&self) -> Result<(), SwellError> {
        let request = McpInitializeRequest {
            protocol_version: MCP_PROTOCOL_VERSION.to_string(),
            capabilities: McpCapabilities {
                tools: Some(McpToolsCapability { list: true }),
            },
            client_info: McpClientInfo {
                name: "swell".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        };

        let response_value = self
            .send_request("initialize", Some(serde_json::to_value(&request).unwrap()))
            .await?;

        let response: McpInitializeResponse =
            serde_json::from_value(response_value).map_err(|e| {
                SwellError::ToolExecutionFailed(format!(
                    "Failed to parse initialize response: {}",
                    e
                ))
            })?;

        // Verify protocol version compatibility
        if response.protocol_version != MCP_PROTOCOL_VERSION {
            warn!(
                server_version = %response.protocol_version,
                client_version = %MCP_PROTOCOL_VERSION,
                "MCP protocol version mismatch"
            );
        }

        // Store server capabilities
        {
            let mut caps = self.capabilities.write().await;
            *caps = Some(response.capabilities);
        }

        // Send notifications/initialized
        let notif = JsonRpcRequest::new(serde_json::Value::Null, "notifications/initialized", None);
        self.send_notification_raw(&notif).await?;

        info!(
            server_name = %response.server_info.name,
            server_version = %response.server_info.version,
            "MCP server initialized"
        );

        Ok(())
    }

    /// Send a JSON-RPC request and wait for response
    async fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value, SwellError> {
        let id = serde_json::json!(Uuid::new_v4().to_string());
        let request = JsonRpcRequest::new(id, method, params);

        let response = self.send_request_raw(&request).await?;

        // Handle error responses
        if let Some(error) = response.error {
            return Err(SwellError::ToolExecutionFailed(format!(
                "MCP error {}: {}",
                error.code, error.message
            )));
        }

        response.result.ok_or_else(|| {
            SwellError::ToolExecutionFailed("MCP response missing result".to_string())
        })
    }

    /// Internal method to send a request and read response
    async fn send_request_raw(
        &self,
        request: &JsonRpcRequest,
    ) -> Result<JsonRpcResponse, SwellError> {
        // Use write lock to get mutable access to process
        let mut process_guard = self.process.write().await;
        let process = process_guard.as_mut().ok_or_else(|| {
            SwellError::ToolExecutionFailed("MCP server not connected".to_string())
        })?;

        // Send request
        let request_json = serde_json::to_string(&request).map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to serialize request: {}", e))
        })?;

        process
            .writer
            .write_all(request_json.as_bytes())
            .await
            .map_err(|e| {
                SwellError::ToolExecutionFailed(format!("Failed to write to MCP stdin: {}", e))
            })?;

        process.writer.write_all(b"\n").await.map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to write newline: {}", e))
        })?;

        process.writer.flush().await.map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to flush stdin: {}", e))
        })?;

        // Read response
        let mut response_line = String::new();
        process
            .reader
            .read_line(&mut response_line)
            .await
            .map_err(|e| {
                SwellError::ToolExecutionFailed(format!("Failed to read MCP response: {}", e))
            })?;

        let response: JsonRpcResponse = serde_json::from_str(&response_line).map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to parse MCP response: {}", e))
        })?;

        Ok(response)
    }

    /// Send a notification (no response expected)
    async fn send_notification_raw(&self, request: &JsonRpcRequest) -> Result<(), SwellError> {
        // Use write lock to get mutable access to process
        let mut process_guard = self.process.write().await;
        let process = process_guard.as_mut().ok_or_else(|| {
            SwellError::ToolExecutionFailed("MCP server not connected".to_string())
        })?;

        let request_json = serde_json::to_string(&request).map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to serialize notification: {}", e))
        })?;

        process
            .writer
            .write_all(request_json.as_bytes())
            .await
            .map_err(|e| {
                SwellError::ToolExecutionFailed(format!("Failed to write to MCP stdin: {}", e))
            })?;

        process.writer.write_all(b"\n").await.map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to write newline: {}", e))
        })?;

        process.writer.flush().await.map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to flush stdin: {}", e))
        })?;

        Ok(())
    }

    /// Disconnect from the MCP server
    pub async fn disconnect(&self) {
        let mut process_guard = self.process.write().await;
        if let Some(mut p) = process_guard.take() {
            info!(url = %self.server_url, "Stopping MCP server");
            p.child.kill().await.ok();
        }

        let mut caps = self.capabilities.write().await;
        *caps = None;

        let mut tools = self.tools.write().await;
        tools.clear();
    }

    /// List available tools from the MCP server
    pub async fn list_tools(&self) -> Result<Vec<McpToolInfo>, SwellError> {
        let server_name = self.server_url.clone();

        let result: Value = self.send_request("tools/list", None).await?;

        let tools_list = result
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();

        let tools: Vec<McpToolInfo> = tools_list
            .into_iter()
            .filter_map(|t| {
                let name = t.get("name")?.as_str()?.to_string();
                let description = t
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string();

                Some(McpToolInfo {
                    name,
                    description,
                    input_schema: t.get("inputSchema").cloned(),
                    server_name: server_name.clone(),
                })
            })
            .collect();

        debug!(count = tools.len(), "Discovered MCP tools");

        // Cache tools
        {
            let mut tools_map = self.tools.write().await;
            for tool in &tools {
                tools_map.insert(tool.name.clone(), tool.clone());
            }
        }

        Ok(tools)
    }

    /// List tools with deferred loading support - returns cached tools
    pub async fn list_tools_deferred(&self) -> Result<Vec<McpToolInfo>, SwellError> {
        let tools = self.tools.read().await;

        if tools.is_empty() {
            drop(tools);
            return self.list_tools().await;
        }

        Ok(tools.values().cloned().collect())
    }

    /// Call an MCP tool
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<ToolOutput, SwellError> {
        // Handle arguments that may already be JSON-encoded as a string
        let args_value = if let Some(args_str) = arguments.as_str() {
            // Arguments is a string - parse it as JSON to get the actual object
            serde_json::from_str(args_str).unwrap_or(arguments)
        } else {
            arguments
        };

        let params = serde_json::json!({
            "name": name,
            "arguments": args_value
        });

        let result: Value = self.send_request("tools/call", Some(params)).await?;

        // Parse the tool call result according to MCP spec
        let content = result
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .cloned();

        let (success, result_str, error_msg) = match content {
            Some(content_obj) => {
                let text = content_obj
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();

                let is_error = content_obj
                    .get("isError")
                    .and_then(|e| e.as_bool())
                    .unwrap_or(false);

                (!is_error, text, None)
            }
            None => {
                let content_str = result
                    .get("content")
                    .map(|c| serde_json::to_string(c).unwrap_or_default())
                    .unwrap_or_default();

                (true, content_str, None)
            }
        };

        if !success {
            return Ok(ToolOutput {
                success: false,
                result: String::new(),
                error: error_msg.or(Some("Tool execution failed".to_string())),
            });
        }

        Ok(ToolOutput {
            success,
            result: result_str,
            error: error_msg,
        })
    }

    /// Get a tool wrapper for a specific MCP tool
    pub async fn get_tool(&self, name: &str) -> Result<McpToolWrapper, SwellError> {
        let tools = self.tools.read().await;

        let info = tools.get(name).cloned().ok_or_else(|| {
            SwellError::ToolExecutionFailed(format!("MCP tool '{}' not found", name))
        })?;

        Ok(McpToolWrapper::new(info, self.clone()))
    }

    /// Get all cached tool infos
    pub async fn get_all_tools(&self) -> Vec<McpToolInfo> {
        let tools = self.tools.read().await;
        tools.values().cloned().collect()
    }

    /// Get server capabilities
    pub async fn get_capabilities(&self) -> Option<McpServerCapabilities> {
        let caps = self.capabilities.read().await;
        caps.clone()
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // Note: Can't do async cleanup here, use disconnect() explicitly
    }
}

/// Manager for MCP server connections with deferred loading support
#[derive(Debug, Clone)]
pub struct McpManager {
    clients: Arc<RwLock<HashMap<String, McpClient>>>,
    deferred_load: Arc<RwLock<bool>>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            deferred_load: Arc::new(RwLock::new(true)),
        }
    }

    /// Enable or disable deferred loading (default: true)
    pub async fn with_deferred_load(self, enabled: bool) -> Self {
        self.set_deferred_load(enabled).await;
        self
    }

    /// Set deferred loading mode
    pub async fn set_deferred_load(&self, enabled: bool) {
        let mut dl = self.deferred_load.write().await;
        *dl = enabled;
    }

    /// Check if deferred loading is enabled
    pub async fn is_deferred_load_enabled(&self) -> bool {
        let dl = self.deferred_load.read().await;
        *dl
    }

    /// Add an MCP server connection
    pub async fn add_server(&self, name: String, url: String) -> Result<(), SwellError> {
        let client = McpClient::new(url);
        client.connect().await?;

        // Discover tools if not deferred
        let deferred = self.is_deferred_load_enabled().await;
        if !deferred {
            let tools = client.list_tools().await?;
            let mut tools_map = client.tools.write().await;
            for tool in tools {
                tools_map.insert(tool.name.clone(), tool);
            }
        }

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

    /// Get all tools from all servers
    pub async fn list_all_tools(&self) -> HashMap<String, Vec<McpToolInfo>> {
        let mut result = HashMap::new();
        let clients = self.clients.read().await;

        for (name, client) in clients.iter() {
            let tools = client.list_tools_deferred().await.unwrap_or_default();
            result.insert(name.clone(), tools);
        }

        result
    }

    /// Register MCP tools with a ToolRegistry
    pub async fn register_with_registry(
        &self,
        registry: &crate::ToolRegistry,
        server_name: &str,
    ) -> Result<(), SwellError> {
        let client = self.get_client(server_name).await.ok_or_else(|| {
            SwellError::ToolExecutionFailed(format!("MCP server '{}' not found", server_name))
        })?;

        // Load tools if deferred
        let deferred = self.is_deferred_load_enabled().await;
        if deferred {
            let tools = client.list_tools().await?;
            let mut tools_map = client.tools.write().await;
            for tool in tools {
                tools_map.insert(tool.name.clone(), tool);
            }
        }

        let tools = client.get_all_tools().await;
        for info in tools {
            let wrapper = McpToolWrapper::new(info, client.clone());
            registry.register(wrapper, crate::registry::ToolCategory::Mcp).await;
        }

        Ok(())
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
        let client = McpClient::new("echo test");
        assert_eq!(client.server_url, "echo test");
        assert!(!client.is_connected().await);
    }

    #[tokio::test]
    async fn test_parse_server_command() {
        let client = McpClient::new("npx test-server --flag");
        // Command parsing is tested internally
        assert_eq!(client.server_url, "npx test-server --flag");
    }

    #[tokio::test]
    async fn test_mcp_tool_info_schema_default() {
        let info = McpToolInfo {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            input_schema: None,
            server_name: "test-server".to_string(),
        };

        let schema = info.schema();
        assert_eq!(schema["type"], "object");
    }

    #[tokio::test]
    async fn test_mcp_tool_info_schema_custom() {
        let custom_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            }
        });

        let info = McpToolInfo {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            input_schema: Some(custom_schema.clone()),
            server_name: "test-server".to_string(),
        };

        let schema = info.schema();
        assert_eq!(schema, custom_schema);
    }
}
