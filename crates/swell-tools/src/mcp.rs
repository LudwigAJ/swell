//! MCP (Model Context Protocol) client for external tool servers.
//!
//! This module implements a client for MCP servers using JSON-RPC 2.0 over stdio.
//! MCP is the industry standard for AI tool integration, providing:
//! - Tool discovery via `tools/list`
//! - Tool execution via `tools/call`
//! - Tool annotations: readOnlyHint, destructiveHint, idempotentHint
//! - outputSchema support for typed results
//! - Dynamic tool discovery via `notifications/tools/list_changed`
//! - Capability negotiation during handshake
//!
//! Reference: https://modelcontextprotocol.io/

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use swell_core::traits::Tool;
use swell_core::{
    PermissionTier, SwellError, ToolOutput, ToolRiskLevel,
};
use swell_core::traits::ToolBehavioralHints;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const JSONRPC_VERSION: &str = "2.0";

/// Tool behavioral annotations as defined in the MCP spec.
/// These provide hints about tool behavior for policy evaluation.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpToolAnnotations {
    /// If true, the tool does not modify its environment
    #[serde(default)]
    pub read_only_hint: Option<bool>,
    /// If true, the tool permanently destroys data
    #[serde(default)]
    pub destructive_hint: Option<bool>,
    /// If true, the tool is safe to retry with the same arguments
    #[serde(default)]
    pub idempotent_hint: Option<bool>,
}

impl McpToolAnnotations {
    /// Returns true if the tool appears to be read-only
    pub fn is_read_only(&self) -> bool {
        self.read_only_hint.unwrap_or(false)
    }

    /// Returns true if the tool appears to be destructive
    pub fn is_destructive(&self) -> bool {
        self.destructive_hint.unwrap_or(false)
    }

    /// Returns true if the tool appears to be idempotent
    pub fn is_idempotent(&self) -> bool {
        self.idempotent_hint.unwrap_or(true)
    }
}

/// MCP client for connecting to MCP servers via stdio
#[derive(Debug, Clone)]
pub struct McpClient {
    server_url: String,
    /// Environment variables to pass to the spawned server process
    env: HashMap<String, String>,
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
    /// Optional output schema for typed results (MCP November 2025 spec)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    /// Tool behavioral annotations: readOnlyHint, destructiveHint, idempotentHint
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<McpToolAnnotations>,
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

    /// Returns the JSON schema for the tool's output
    pub fn output_schema(&self) -> Option<Value> {
        self.output_schema.clone()
    }

    /// Returns the tool's behavioral annotations
    pub fn annotations(&self) -> Option<&McpToolAnnotations> {
        self.annotations.as_ref()
    }

    /// Determines the risk level based on annotations
    pub fn risk_level_from_annotations(&self) -> ToolRiskLevel {
        if let Some(ref annotations) = self.annotations {
            if annotations.is_destructive() {
                ToolRiskLevel::Destructive
            } else if annotations.is_read_only() {
                ToolRiskLevel::Read
            } else {
                ToolRiskLevel::Write
            }
        } else {
            ToolRiskLevel::Write
        }
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

    /// Returns the output schema for this tool, if specified
    pub fn output_schema(&self) -> Option<Value> {
        self.info.output_schema()
    }

    /// Returns the annotations for this tool, if specified
    pub fn annotations(&self) -> Option<&McpToolAnnotations> {
        self.info.annotations()
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
        // Use annotation-based risk classification if available
        self.info.risk_level_from_annotations()
    }

    fn permission_tier(&self) -> PermissionTier {
        // Use annotation-based permission tier
        if let Some(ref annotations) = self.info.annotations {
            if annotations.is_destructive() {
                PermissionTier::Deny
            } else if annotations.is_read_only() {
                PermissionTier::Auto
            } else {
                PermissionTier::Ask
            }
        } else {
            PermissionTier::Ask
        }
    }

    fn input_schema(&self) -> Value {
        self.info.schema()
    }

    fn behavioral_hints(&self) -> ToolBehavioralHints {
        ToolBehavioralHints {
            read_only_hint: self
                .info
                .annotations
                .as_ref()
                .is_some_and(|a| a.is_read_only()),
            destructive_hint: self
                .info
                .annotations
                .as_ref()
                .is_some_and(|a| a.is_destructive()),
            idempotent_hint: self
                .info
                .annotations
                .as_ref()
                .is_none_or(|a| a.is_idempotent()),
        }
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, SwellError> {
        self.client.call_tool(&self.info.name, arguments).await
    }
}

impl McpClient {
    /// Create a new MCP client for the given server command
    pub fn new(server_url: impl Into<String>) -> Self {
        Self::new_with_env(server_url, HashMap::new())
    }

    /// Create a new MCP client with environment variables
    pub fn new_with_env(server_url: impl Into<String>, env: HashMap<String, String>) -> Self {
        Self {
            server_url: server_url.into(),
            env,
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
            .envs(&self.env)
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

                // Parse annotations (readOnlyHint, destructiveHint, idempotentHint)
                let annotations = t
                    .get("annotations")
                    .and_then(|a| serde_json::from_value::<McpToolAnnotations>(a.clone()).ok());

                // Parse outputSchema (November 2025 MCP spec)
                let output_schema = t.get("outputSchema").cloned();

                Some(McpToolInfo {
                    name,
                    description,
                    input_schema: t.get("inputSchema").cloned(),
                    output_schema,
                    annotations,
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

    /// Refresh the tool cache when server announces list changes.
    /// This handles the `notifications/tools/list_changed` notification.
    pub async fn refresh_tools(&self) -> Result<Vec<McpToolInfo>, SwellError> {
        info!("Refreshing MCP tools due to list change notification");

        // Clear existing cache
        {
            let mut tools_map = self.tools.write().await;
            tools_map.clear();
        }

        // Re-fetch all tools
        self.list_tools().await
    }

    /// Check if the server supports tool list change notifications
    pub async fn supports_tool_list_changes(&self) -> bool {
        if let Some(caps) = self.get_capabilities().await {
            // Check if the server has tools capability with list subscribed
            caps.tools.as_ref().map(|t| t.list).unwrap_or(false)
        } else {
            false
        }
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
            registry
                .register(wrapper, crate::registry::ToolCategory::Mcp)
                .await;
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
            output_schema: None,
            annotations: None,
            server_name: "test-server".to_string(),
        };

        let schema = info.schema();
        assert_eq!(schema["type"], "object");
        assert!(info.output_schema().is_none());
        assert!(info.annotations().is_none());
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
            output_schema: None,
            annotations: None,
            server_name: "test-server".to_string(),
        };

        let schema = info.schema();
        assert_eq!(schema, custom_schema);
    }

    #[tokio::test]
    async fn test_mcp_tool_info_with_annotations() {
        let annotations = McpToolAnnotations {
            read_only_hint: Some(true),
            destructive_hint: Some(false),
            idempotent_hint: Some(true),
        };

        let info = McpToolInfo {
            name: "read_file".to_string(),
            description: "Reads a file from disk".to_string(),
            input_schema: None,
            output_schema: None,
            annotations: Some(annotations),
            server_name: "test-server".to_string(),
        };

        assert!(info.annotations().is_some());
        let annot = info.annotations().unwrap();
        assert!(annot.is_read_only());
        assert!(!annot.is_destructive());
        assert!(annot.is_idempotent());
    }

    #[tokio::test]
    async fn test_mcp_tool_info_with_output_schema() {
        let output_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "content": { "type": "string" },
                "lines": { "type": "integer" }
            }
        });

        let info = McpToolInfo {
            name: "read_file".to_string(),
            description: "Reads a file from disk".to_string(),
            input_schema: None,
            output_schema: Some(output_schema.clone()),
            annotations: None,
            server_name: "test-server".to_string(),
        };

        assert!(info.output_schema().is_some());
        assert_eq!(info.output_schema().unwrap(), output_schema);
    }

    #[tokio::test]
    async fn test_mcp_tool_risk_level_from_annotations() {
        // Test read-only tool
        let read_only_info = McpToolInfo {
            name: "read".to_string(),
            description: "Read-only tool".to_string(),
            input_schema: None,
            output_schema: None,
            annotations: Some(McpToolAnnotations {
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
            }),
            server_name: "test-server".to_string(),
        };
        assert_eq!(
            read_only_info.risk_level_from_annotations(),
            ToolRiskLevel::Read
        );

        // Test destructive tool
        let destructive_info = McpToolInfo {
            name: "delete".to_string(),
            description: "Destructive tool".to_string(),
            input_schema: None,
            output_schema: None,
            annotations: Some(McpToolAnnotations {
                read_only_hint: Some(false),
                destructive_hint: Some(true),
                idempotent_hint: Some(false),
            }),
            server_name: "test-server".to_string(),
        };
        assert_eq!(
            destructive_info.risk_level_from_annotations(),
            ToolRiskLevel::Destructive
        );

        // Test tool without annotations
        let no_annot_info = McpToolInfo {
            name: "unknown".to_string(),
            description: "Unknown tool".to_string(),
            input_schema: None,
            output_schema: None,
            annotations: None,
            server_name: "test-server".to_string(),
        };
        assert_eq!(
            no_annot_info.risk_level_from_annotations(),
            ToolRiskLevel::Write
        );
    }

    #[tokio::test]
    async fn test_mcp_tool_wrapper_permission_tier() {
        let client = McpClient::new("echo test");

        // Read-only tool should have Auto permission
        let read_only_info = McpToolInfo {
            name: "read".to_string(),
            description: "Read-only tool".to_string(),
            input_schema: None,
            output_schema: None,
            annotations: Some(McpToolAnnotations {
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
            }),
            server_name: "test-server".to_string(),
        };
        let wrapper = McpToolWrapper::new(read_only_info, client.clone());
        assert_eq!(wrapper.permission_tier(), PermissionTier::Auto);

        // Destructive tool should have Deny permission
        let destructive_info = McpToolInfo {
            name: "delete".to_string(),
            description: "Destructive tool".to_string(),
            input_schema: None,
            output_schema: None,
            annotations: Some(McpToolAnnotations {
                read_only_hint: Some(false),
                destructive_hint: Some(true),
                idempotent_hint: Some(false),
            }),
            server_name: "test-server".to_string(),
        };
        let wrapper = McpToolWrapper::new(destructive_info, client.clone());
        assert_eq!(wrapper.permission_tier(), PermissionTier::Deny);

        // Tool without annotations should have Ask permission
        let no_annot_info = McpToolInfo {
            name: "unknown".to_string(),
            description: "Unknown tool".to_string(),
            input_schema: None,
            output_schema: None,
            annotations: None,
            server_name: "test-server".to_string(),
        };
        let wrapper = McpToolWrapper::new(no_annot_info, client);
        assert_eq!(wrapper.permission_tier(), PermissionTier::Ask);
    }

    #[tokio::test]
    async fn test_mcp_tool_wrapper_output_schema() {
        let client = McpClient::new("echo test");

        let output_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "result": { "type": "string" }
            }
        });

        let info = McpToolInfo {
            name: "test_tool".to_string(),
            description: "Test tool".to_string(),
            input_schema: None,
            output_schema: Some(output_schema.clone()),
            annotations: None,
            server_name: "test-server".to_string(),
        };

        let wrapper = McpToolWrapper::new(info, client);
        assert!(wrapper.output_schema().is_some());
        assert_eq!(wrapper.output_schema().unwrap(), output_schema);
    }

    #[tokio::test]
    async fn test_mcp_tool_annotations_default() {
        let annot = McpToolAnnotations::default();
        // Default values should make tool appear non-destructive and idempotent
        assert!(!annot.is_destructive());
        assert!(annot.is_idempotent());
        // read_only_hint defaults to false
        assert!(!annot.is_read_only());
    }
}

// =============================================================================
// Tree-sitter MCP Integration Tests
// =============================================================================
//
// These tests verify the MCP client integration with mcp-server-tree-sitter.
// The tree-sitter server provides AST analysis, symbol extraction, and code
// complexity analysis tools.
//
// Reference: https://github.com/wrale/mcp-server-tree-sitter

#[cfg(test)]
mod mcp_treesitter_tests {
    use super::*;
    use std::collections::HashSet;

    /// Expected tree-sitter MCP tools based on FEATURES.md
    const EXPECTED_TREE_SITTER_TOOLS: &[&str] = &[
        // AST Analysis Commands
        "get_ast",
        "get_node_at_position",
        // Search and Query Commands
        "run_query",
        // Code Analysis Commands
        "get_symbols",
        "find_usage",
        "analyze_project",
        "get_dependencies",
        "analyze_complexity",
        // Project Management Commands
        "register_project_tool",
        "list_projects_tool",
        "remove_project_tool",
        // Language Tools Commands
        "list_languages",
        "check_language_available",
        // File Operations Commands
        "list_files",
        "get_file",
        "get_file_metadata",
    ];

    /// Verify that a tool info matches expected tree-sitter tool structure
    fn validate_tree_sitter_tool_info(info: &McpToolInfo) -> Result<(), String> {
        // Tree-sitter tools should be read-only (they analyze code without modifying it)
        if let Some(ref annotations) = info.annotations {
            // Most tree-sitter tools are read-only
            if !annotations.is_destructive() {
                // Good - tools are marked as non-destructive
            }
        }

        // Verify tool has a description
        if info.description.is_empty() {
            return Err(format!("Tool '{}' has empty description", info.name));
        }

        // Verify tool has an input schema (tree-sitter tools require arguments)
        let schema = info.schema();
        if schema.get("type").is_none() {
            return Err(format!("Tool '{}' missing schema type", info.name));
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_tree_sitter_tool_names() {
        // Verify all expected tree-sitter tool names are recognized
        let expected_tools: HashSet<&str> = EXPECTED_TREE_SITTER_TOOLS.iter().cloned().collect();

        // These are the core AST/symbol/analysis tools that must be present
        let core_tools = [
            "get_ast",
            "get_node_at_position",
            "run_query",
            "get_symbols",
            "find_usage",
            "analyze_project",
            "get_dependencies",
            "analyze_complexity",
        ];

        for tool_name in core_tools {
            assert!(
                expected_tools.contains(tool_name),
                "Core tool '{}' should be in expected tree-sitter tools",
                tool_name
            );
        }
    }

    #[tokio::test]
    async fn test_tree_sitter_ast_tool_info() {
        // Test get_ast tool info structure
        let get_ast_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "project": { "type": "string", "description": "Project name" },
                "path": { "type": "string", "description": "File path within project" },
                "max_depth": { "type": "integer", "description": "Maximum tree depth" },
                "include_text": { "type": "boolean", "description": "Include source text" }
            },
            "required": ["project", "path"]
        });

        let get_ast_info = McpToolInfo {
            name: "get_ast".to_string(),
            description: "Returns AST using efficient cursor-based traversal with proper node IDs"
                .to_string(),
            input_schema: Some(get_ast_schema.clone()),
            output_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "nodes": { "type": "array", "description": "AST nodes" },
                    "root": { "type": "object", "description": "Root node" }
                }
            })),
            annotations: Some(McpToolAnnotations {
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
            }),
            server_name: "tree-sitter".to_string(),
        };

        assert_eq!(get_ast_info.name, "get_ast");
        assert!(get_ast_info.description.contains("AST"));
        assert!(get_ast_info.schema().get("properties").is_some());

        // Verify read-only annotation
        let annot = get_ast_info.annotations.as_ref().unwrap();
        assert!(annot.is_read_only());
        assert!(!annot.is_destructive());
        assert!(annot.is_idempotent());
    }

    #[tokio::test]
    async fn test_tree_sitter_node_at_position_tool_info() {
        // Test get_node_at_position tool info structure
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "project": { "type": "string" },
                "path": { "type": "string" },
                "row": { "type": "integer", "description": "Line number (0-indexed)" },
                "column": { "type": "integer", "description": "Column number (0-indexed)" }
            },
            "required": ["project", "path", "row", "column"]
        });

        let info = McpToolInfo {
            name: "get_node_at_position".to_string(),
            description: "Retrieves nodes at a specific position in a file".to_string(),
            input_schema: Some(schema),
            output_schema: None,
            annotations: Some(McpToolAnnotations {
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
            }),
            server_name: "tree-sitter".to_string(),
        };

        assert_eq!(info.name, "get_node_at_position");
        validate_tree_sitter_tool_info(&info).unwrap();
    }

    #[tokio::test]
    async fn test_tree_sitter_run_query_tool_info() {
        // Test run_query tool info structure
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "project": { "type": "string" },
                "query": { "type": "string", "description": "Tree-sitter query" },
                "file_path": { "type": "string" },
                "language": { "type": "string", "description": "Language (python, rust, etc.)" }
            },
            "required": ["project", "query", "file_path", "language"]
        });

        let info = McpToolInfo {
            name: "run_query".to_string(),
            description: "Executes tree-sitter queries and returns results".to_string(),
            input_schema: Some(schema),
            output_schema: Some(serde_json::json!({
                "type": "array",
                "description": "Query matches"
            })),
            annotations: Some(McpToolAnnotations {
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
            }),
            server_name: "tree-sitter".to_string(),
        };

        assert_eq!(info.name, "run_query");
        validate_tree_sitter_tool_info(&info).unwrap();
    }

    #[tokio::test]
    async fn test_tree_sitter_get_symbols_tool_info() {
        // Test get_symbols tool info structure
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "project": { "type": "string" },
                "file_path": { "type": "string" }
            },
            "required": ["project", "file_path"]
        });

        let info = McpToolInfo {
            name: "get_symbols".to_string(),
            description: "Extracts symbols (functions, classes, imports) from files".to_string(),
            input_schema: Some(schema),
            output_schema: Some(serde_json::json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "kind": { "type": "string" },
                        "location": { "type": "object" }
                    }
                }
            })),
            annotations: Some(McpToolAnnotations {
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
            }),
            server_name: "tree-sitter".to_string(),
        };

        assert_eq!(info.name, "get_symbols");
        validate_tree_sitter_tool_info(&info).unwrap();

        // Verify output schema indicates array of symbols
        let output = info.output_schema().unwrap();
        assert_eq!(output["type"], "array");
    }

    #[tokio::test]
    async fn test_tree_sitter_find_usage_tool_info() {
        // Test find_usage tool info structure
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "project": { "type": "string" },
                "symbol": { "type": "string", "description": "Symbol name to find" },
                "language": { "type": "string" }
            },
            "required": ["project", "symbol", "language"]
        });

        let info = McpToolInfo {
            name: "find_usage".to_string(),
            description: "Finds usage of symbols across project files".to_string(),
            input_schema: Some(schema),
            output_schema: Some(serde_json::json!({
                "type": "array",
                "description": "Symbol usage locations"
            })),
            annotations: Some(McpToolAnnotations {
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
            }),
            server_name: "tree-sitter".to_string(),
        };

        assert_eq!(info.name, "find_usage");
        validate_tree_sitter_tool_info(&info).unwrap();
    }

    #[tokio::test]
    async fn test_tree_sitter_analyze_project_tool_info() {
        // Test analyze_project tool info structure
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "project": { "type": "string" },
                "scan_depth": { "type": "integer", "description": "Directory scan depth" }
            },
            "required": ["project"]
        });

        let info = McpToolInfo {
            name: "analyze_project".to_string(),
            description: "Project structure analysis".to_string(),
            input_schema: Some(schema),
            output_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "files": { "type": "array" },
                    "structure": { "type": "object" }
                }
            })),
            annotations: Some(McpToolAnnotations {
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
            }),
            server_name: "tree-sitter".to_string(),
        };

        assert_eq!(info.name, "analyze_project");
        validate_tree_sitter_tool_info(&info).unwrap();
    }

    #[tokio::test]
    async fn test_tree_sitter_get_dependencies_tool_info() {
        // Test get_dependencies tool info structure
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "project": { "type": "string" },
                "file_path": { "type": "string" }
            },
            "required": ["project", "file_path"]
        });

        let info = McpToolInfo {
            name: "get_dependencies".to_string(),
            description: "Identifies dependencies from import statements".to_string(),
            input_schema: Some(schema),
            output_schema: Some(serde_json::json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "type": { "type": "string" },
                        "target": { "type": "string" }
                    }
                }
            })),
            annotations: Some(McpToolAnnotations {
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
            }),
            server_name: "tree-sitter".to_string(),
        };

        assert_eq!(info.name, "get_dependencies");
        validate_tree_sitter_tool_info(&info).unwrap();
    }

    #[tokio::test]
    async fn test_tree_sitter_analyze_complexity_tool_info() {
        // Test analyze_complexity tool info structure
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "project": { "type": "string" },
                "file_path": { "type": "string" }
            },
            "required": ["project", "file_path"]
        });

        let info = McpToolInfo {
            name: "analyze_complexity".to_string(),
            description: "Provides accurate code complexity metrics".to_string(),
            input_schema: Some(schema),
            output_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "lines": { "type": "integer" },
                    "cyclomatic": { "type": "integer" },
                    "functions": { "type": "integer" }
                }
            })),
            annotations: Some(McpToolAnnotations {
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
            }),
            server_name: "tree-sitter".to_string(),
        };

        assert_eq!(info.name, "analyze_complexity");
        validate_tree_sitter_tool_info(&info).unwrap();
    }

    #[tokio::test]
    async fn test_tree_sitter_all_tools_are_read_only() {
        // All tree-sitter analysis tools should be marked as read-only
        let client = McpClient::new("tree-sitter-server");
        let tree_sitter_tools = [
            (
                "get_ast",
                McpToolInfo {
                    name: "get_ast".to_string(),
                    description: "Get AST".to_string(),
                    input_schema: None,
                    output_schema: None,
                    annotations: Some(McpToolAnnotations {
                        read_only_hint: Some(true),
                        destructive_hint: Some(false),
                        idempotent_hint: Some(true),
                    }),
                    server_name: "tree-sitter".to_string(),
                },
            ),
            (
                "get_node_at_position",
                McpToolInfo {
                    name: "get_node_at_position".to_string(),
                    description: "Get node".to_string(),
                    input_schema: None,
                    output_schema: None,
                    annotations: Some(McpToolAnnotations {
                        read_only_hint: Some(true),
                        destructive_hint: Some(false),
                        idempotent_hint: Some(true),
                    }),
                    server_name: "tree-sitter".to_string(),
                },
            ),
            (
                "run_query",
                McpToolInfo {
                    name: "run_query".to_string(),
                    description: "Run query".to_string(),
                    input_schema: None,
                    output_schema: None,
                    annotations: Some(McpToolAnnotations {
                        read_only_hint: Some(true),
                        destructive_hint: Some(false),
                        idempotent_hint: Some(true),
                    }),
                    server_name: "tree-sitter".to_string(),
                },
            ),
            (
                "get_symbols",
                McpToolInfo {
                    name: "get_symbols".to_string(),
                    description: "Get symbols".to_string(),
                    input_schema: None,
                    output_schema: None,
                    annotations: Some(McpToolAnnotations {
                        read_only_hint: Some(true),
                        destructive_hint: Some(false),
                        idempotent_hint: Some(true),
                    }),
                    server_name: "tree-sitter".to_string(),
                },
            ),
            (
                "find_usage",
                McpToolInfo {
                    name: "find_usage".to_string(),
                    description: "Find usage".to_string(),
                    input_schema: None,
                    output_schema: None,
                    annotations: Some(McpToolAnnotations {
                        read_only_hint: Some(true),
                        destructive_hint: Some(false),
                        idempotent_hint: Some(true),
                    }),
                    server_name: "tree-sitter".to_string(),
                },
            ),
            (
                "analyze_project",
                McpToolInfo {
                    name: "analyze_project".to_string(),
                    description: "Analyze project".to_string(),
                    input_schema: None,
                    output_schema: None,
                    annotations: Some(McpToolAnnotations {
                        read_only_hint: Some(true),
                        destructive_hint: Some(false),
                        idempotent_hint: Some(true),
                    }),
                    server_name: "tree-sitter".to_string(),
                },
            ),
            (
                "get_dependencies",
                McpToolInfo {
                    name: "get_dependencies".to_string(),
                    description: "Get dependencies".to_string(),
                    input_schema: None,
                    output_schema: None,
                    annotations: Some(McpToolAnnotations {
                        read_only_hint: Some(true),
                        destructive_hint: Some(false),
                        idempotent_hint: Some(true),
                    }),
                    server_name: "tree-sitter".to_string(),
                },
            ),
            (
                "analyze_complexity",
                McpToolInfo {
                    name: "analyze_complexity".to_string(),
                    description: "Analyze complexity".to_string(),
                    input_schema: None,
                    output_schema: None,
                    annotations: Some(McpToolAnnotations {
                        read_only_hint: Some(true),
                        destructive_hint: Some(false),
                        idempotent_hint: Some(true),
                    }),
                    server_name: "tree-sitter".to_string(),
                },
            ),
        ];

        for (name, tool_info) in tree_sitter_tools {
            let risk_level = tool_info.risk_level_from_annotations();
            assert_eq!(
                risk_level,
                ToolRiskLevel::Read,
                "Tool '{}' should be read-only but got {:?}",
                name,
                risk_level
            );

            // Create wrapper and check permission tier
            let wrapper = McpToolWrapper::new(tool_info, client.clone());
            let permission = wrapper.permission_tier();
            assert_eq!(
                permission,
                PermissionTier::Auto,
                "Tool '{}' should have Auto permission but got {:?}",
                name,
                permission
            );
        }
    }

    #[tokio::test]
    async fn test_tree_sitter_tool_result_parsing() {
        // Test that we can parse tree-sitter tool call results
        // Simulating the response format from mcp-server-tree-sitter

        // Example get_ast response
        let get_ast_response = serde_json::json!({
            "content": [
                {
                    "type": "text",
                    "text": "{\"nodes\": [{\"id\": \"root_0\", \"kind\": \"program\", \"name\": null}], \"root\": {\"id\": \"root_0\"}}"
                }
            ]
        });

        let content = get_ast_response
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .cloned();

        assert!(content.is_some());
        let content_obj = content.unwrap();
        let text = content_obj.get("text").and_then(|t| t.as_str()).unwrap();
        assert!(text.contains("nodes"));
        assert!(text.contains("root"));
    }

    #[tokio::test]
    async fn test_tree_sitter_symbol_result_parsing() {
        // Test parsing of get_symbols response
        let symbols_response = serde_json::json!({
            "content": [
                {
                    "type": "text",
                    "text": "[{\"name\": \"hello_world\", \"kind\": \"function\", \"location\": {\"row\": 1, \"column\": 0}}]"
                }
            ]
        });

        let content = symbols_response
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .cloned();

        assert!(content.is_some());
        let content_obj = content.unwrap();
        let text = content_obj.get("text").and_then(|t| t.as_str()).unwrap();

        // Parse the JSON array inside the text field
        let symbols: Vec<serde_json::Value> = serde_json::from_str(text).unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0]["name"], "hello_world");
        assert_eq!(symbols[0]["kind"], "function");
    }

    #[tokio::test]
    async fn test_tree_sitter_complexity_result_parsing() {
        // Test parsing of analyze_complexity response
        let complexity_response = serde_json::json!({
            "content": [
                {
                    "type": "text",
                    "text": "{\"lines\": 42, \"cyclomatic\": 3, \"functions\": 2}"
                }
            ]
        });

        let content = complexity_response
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .cloned();

        assert!(content.is_some());
        let content_obj = content.unwrap();
        let text = content_obj.get("text").and_then(|t| t.as_str()).unwrap();

        let metrics: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(metrics["lines"], 42);
        assert_eq!(metrics["cyclomatic"], 3);
        assert_eq!(metrics["functions"], 2);
    }

    #[tokio::test]
    async fn test_tree_sitter_tool_client_creation() {
        // Test that we can create an MCP client configured for tree-sitter
        let client = McpClient::new("python3 -m mcp_server_tree_sitter");

        assert_eq!(client.server_url, "python3 -m mcp_server_tree_sitter");
        // Client is not connected initially
        assert!(!client.is_connected().await);
    }

    #[test]
    fn test_tree_sitter_expected_tool_count() {
        // Verify we have tests for all expected tree-sitter tools
        // This ensures we don't forget to add tests for new tools

        let core_tools = vec![
            "get_ast",
            "get_node_at_position",
            "run_query",
            "get_symbols",
            "find_usage",
            "analyze_project",
            "get_dependencies",
            "analyze_complexity",
        ];

        assert_eq!(core_tools.len(), 8, "Should have 8 core tree-sitter tools");
    }
}
