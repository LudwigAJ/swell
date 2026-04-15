//! MCP Server Configuration System
//!
//! This module provides configuration loading and management for MCP servers,
//! supporting lazy server startup, health checks, and reconnection logic.
//!
//! Configuration is loaded from `.swell/mcp_servers.json` with the following format:
//! ```json
//! {
//!   "servers": [
//!     {
//!       "name": "tree-sitter",
//!       "command": "python3",
//!       "args": ["-m", "mcp_server_tree_sitter"],
//!       "env": {}
//!     }
//!   ]
//! }
//! ```
//!
//! Lazy startup means servers only start when their tools are actually needed,
//! reducing resource usage and avoiding unnecessary server processes.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use swell_core::SwellError;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use super::mcp::{McpClient, McpConnectionError, McpToolInfo};

/// Configuration for a single MCP server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Unique name for this server (used for identification)
    pub name: String,
    /// The command to execute (e.g., "python3", "npx", "node")
    pub command: String,
    /// Arguments to pass to the command
    pub args: Vec<String>,
    /// Environment variables to set for the server process
    #[serde(default)]
    pub env: HashMap<String, String>,
}

impl McpServerConfig {
    /// Convert config into a command string for McpClient
    pub fn to_command_string(&self) -> String {
        let mut cmd = self.command.clone();
        for arg in &self.args {
            cmd.push(' ');
            cmd.push_str(arg);
        }
        cmd
    }

    /// Check if this server config is valid
    pub fn is_valid(&self) -> bool {
        !self.name.is_empty() && !self.command.is_empty()
    }
}

/// Root configuration structure for mcp_servers.json
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpServersConfig {
    /// List of server configurations
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

impl McpServersConfig {
    /// Load configuration from a JSON file
    pub async fn load_from_file(path: &Path) -> Result<Self, SwellError> {
        let content = tokio::fs::read_to_string(path).await.map_err(|e| {
            SwellError::ConfigError(format!("Failed to read MCP config file: {}", e))
        })?;

        Self::load_from_str(&content)
    }

    /// Load configuration from a JSON string
    pub fn load_from_str(content: &str) -> Result<Self, SwellError> {
        serde_json::from_str(content)
            .map_err(|e| SwellError::ConfigError(format!("Failed to parse MCP config: {}", e)))
    }

    /// Get a server config by name
    pub fn get_server(&self, name: &str) -> Option<&McpServerConfig> {
        self.servers.iter().find(|s| s.name == name)
    }

    /// Get all server names
    pub fn server_names(&self) -> Vec<&str> {
        self.servers.iter().map(|s| s.name.as_str()).collect()
    }
}

/// Health status of an MCP server
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum McpServerHealth {
    /// Server is connected and responsive
    Healthy,
    /// Server is starting up
    Starting,
    /// Server is not connected
    #[default]
    Disconnected,
    /// Server is attempting to reconnect
    Reconnecting,
    /// Server is degraded (connected but with limited functionality or warnings)
    Degraded,
    /// Server has failed after max reconnection attempts
    Failed,
}

impl McpServerHealth {
    /// Returns true if this health status indicates the server is healthy
    pub fn is_healthy(&self) -> bool {
        matches!(self, McpServerHealth::Healthy)
    }

    /// Returns true if this health status indicates the server has failed or is degraded
    pub fn is_failed(&self) -> bool {
        matches!(self, McpServerHealth::Failed | McpServerHealth::Degraded)
    }
}

/// State tracking for a single MCP server
#[derive(Debug, Clone)]
pub struct McpServerState {
    /// Server configuration
    pub config: McpServerConfig,
    /// Current health status
    pub health: McpServerHealth,
    /// Number of reconnection attempts
    pub reconnect_attempts: u32,
    /// Last successful connection timestamp (Unix epoch millis)
    pub last_connected_ms: Option<u64>,
}

impl McpServerState {
    pub fn new(config: McpServerConfig) -> Self {
        Self {
            config,
            health: McpServerHealth::Disconnected,
            reconnect_attempts: 0,
            last_connected_ms: None,
        }
    }
}

/// Configuration for server health check and reconnection behavior
#[derive(Debug, Clone)]
pub struct McpReconnectConfig {
    /// Maximum number of reconnection attempts before marking as Failed
    pub max_reconnect_attempts: u32,
    /// Delay between reconnection attempts in milliseconds
    pub reconnect_delay_ms: u64,
    /// Health check interval in milliseconds
    pub health_check_interval_ms: u64,
    /// Enable auto-reconnection
    pub auto_reconnect: bool,
}

impl Default for McpReconnectConfig {
    fn default() -> Self {
        Self {
            max_reconnect_attempts: 3,
            reconnect_delay_ms: 1000,
            health_check_interval_ms: 30000,
            auto_reconnect: true,
        }
    }
}

/// Manager for MCP server configuration with lazy startup and health monitoring
#[derive(Debug)]
pub struct McpConfigManager {
    /// Configuration loaded from file
    config: McpServersConfig,
    /// Server states (health, reconnect attempts, etc.)
    server_states: Arc<RwLock<HashMap<String, McpServerState>>>,
    /// Active MCP clients (lazy-started)
    clients: Arc<RwLock<HashMap<String, McpClient>>>,
    /// Reconnection configuration
    reconnect_config: McpReconnectConfig,
    /// Deferred loading mode (servers start only when tools needed)
    deferred_load: Arc<RwLock<bool>>,
}

impl McpConfigManager {
    /// Create a new config manager from a config file path
    pub async fn new(config_path: &Path) -> Result<Self, SwellError> {
        let config = McpServersConfig::load_from_file(config_path).await?;

        let base_dir = config_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        let _ = base_dir; // suppress unused warning

        Ok(Self {
            config,
            server_states: Arc::new(RwLock::new(HashMap::new())),
            clients: Arc::new(RwLock::new(HashMap::new())),
            reconnect_config: McpReconnectConfig::default(),
            deferred_load: Arc::new(RwLock::new(true)),
        })
    }

    /// Create a new config manager from a config string
    pub fn new_from_str(config_content: &str) -> Result<Self, SwellError> {
        let config = McpServersConfig::load_from_str(config_content)?;

        Ok(Self {
            config,
            server_states: Arc::new(RwLock::new(HashMap::new())),
            clients: Arc::new(RwLock::new(HashMap::new())),
            reconnect_config: McpReconnectConfig::default(),
            deferred_load: Arc::new(RwLock::new(true)),
        })
    }

    /// Get the underlying configuration
    pub fn config(&self) -> &McpServersConfig {
        &self.config
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

    /// Get the reconnect configuration
    pub fn reconnect_config(&self) -> &McpReconnectConfig {
        &self.reconnect_config
    }

    /// Set reconnect configuration
    pub fn with_reconnect_config(mut self, config: McpReconnectConfig) -> Self {
        self.reconnect_config = config;
        self
    }

    /// Get the health status of a server
    pub async fn get_server_health(&self, name: &str) -> McpServerHealth {
        let states = self.server_states.read().await;
        states
            .get(name)
            .map(|s| s.health)
            .unwrap_or(McpServerHealth::Disconnected)
    }

    /// Get health status for all servers (including configured but not started)
    pub async fn get_all_health(&self) -> HashMap<String, McpServerHealth> {
        let states = self.server_states.read().await;
        let mut result = HashMap::new();

        // First add all configured servers with their current state
        for server_config in &self.config.servers {
            let health = states
                .get(&server_config.name)
                .map(|s| s.health)
                .unwrap_or(McpServerHealth::Disconnected);
            result.insert(server_config.name.clone(), health);
        }

        // Add any servers that might be in state but not in config (shouldn't happen but for safety)
        for (name, state) in states.iter() {
            result.entry(name.clone()).or_insert(state.health);
        }

        result
    }

    /// Get reconnection state for a server
    pub async fn get_reconnect_attempts(&self, name: &str) -> u32 {
        let states = self.server_states.read().await;
        states.get(name).map(|s| s.reconnect_attempts).unwrap_or(0)
    }

    /// Get the client for a server, starting it if needed (lazy startup)
    pub async fn get_or_start_server(&self, name: &str) -> Result<McpClient, SwellError> {
        // Check if we have an active client
        {
            let clients = self.clients.read().await;
            if let Some(client) = clients.get(name) {
                if client.is_connected().await {
                    return Ok(client.clone());
                }
            }
        }

        // Server not started or disconnected - start it now
        self.start_server(name).await
    }

    /// Start a specific server with retry logic based on failure classification.
    ///
    /// Recoverable errors (timeout, connection refused) are retried with exponential
    /// backoff up to `max_reconnect_attempts`. Non-recoverable errors (binary not found,
    /// protocol mismatch) fail immediately without retry.
    ///
    /// Reference: VAL-MCP-003
    pub async fn start_server(&self, name: &str) -> Result<McpClient, SwellError> {
        // Get server config
        let server_config = self
            .config
            .get_server(name)
            .ok_or_else(|| {
                SwellError::ConfigError(format!("MCP server '{}' not found in config", name))
            })?
            .clone();

        // Update state to starting
        {
            let mut states = self.server_states.write().await;
            let state = states
                .entry(name.to_string())
                .or_insert_with(|| McpServerState::new(server_config.clone()));
            state.health = McpServerHealth::Starting;
        }

        info!(server = %name, "Starting MCP server (lazy startup)");

        // Create client with the command string and environment variables
        let client = McpClient::new_with_env(server_config.to_command_string(), server_config.env);

        // Mark ServerRegistration phase as complete before connecting
        // (ServerRegistration happens in McpConfigManager when registering in server_states)
        client.mark_server_registered().await;

        // Attempt connection with retry logic for recoverable errors
        let result = self
            .connect_with_retry(&client, name)
            .await;

        match result {
            Ok(()) => {
                // Update state to healthy
                let mut states = self.server_states.write().await;
                if let Some(state) = states.get_mut(name) {
                    state.health = McpServerHealth::Healthy;
                    state.reconnect_attempts = 0;
                    state.last_connected_ms = Some(chrono::Utc::now().timestamp_millis() as u64);
                }

                // Store client
                let mut clients = self.clients.write().await;
                clients.insert(name.to_string(), client.clone());

                info!(server = %name, "MCP server started successfully");
                Ok(client)
            }
            Err(e) => {
                // Update state to disconnected or failed based on error classification
                let mut states = self.server_states.write().await;
                if let Some(state) = states.get_mut(name) {
                    // If we've exhausted retries or error is non-recoverable, mark as failed
                    if state.reconnect_attempts >= self.reconnect_config.max_reconnect_attempts
                        || !e.is_recoverable()
                    {
                        state.health = McpServerHealth::Failed;
                    } else {
                        state.health = McpServerHealth::Disconnected;
                    }
                }

                error!(server = %name, error = %e, "Failed to start MCP server");
                Err(SwellError::ToolExecutionFailed(e.to_string()))
            }
        }
    }

    /// Connect with retry logic based on failure classification.
    ///
    /// For recoverable errors, retries with exponential backoff up to max_attempts.
    /// For non-recoverable errors, returns immediately without retry.
    ///
    /// Reference: VAL-MCP-003
    async fn connect_with_retry(
        &self,
        client: &McpClient,
        name: &str,
    ) -> Result<(), McpConnectionError> {
        let max_attempts = self.reconnect_config.max_reconnect_attempts;
        let base_delay_ms = self.reconnect_config.reconnect_delay_ms;

        let mut attempt = 0;

        loop {
            // Attempt connection
            match client.connect_with_classification().await {
                Ok(()) => return Ok(()),
                Err(connection_error) => {
                    // Check if error is recoverable
                    if !connection_error.is_recoverable() {
                        // Non-recoverable error - fail immediately
                        warn!(
                            server = %name,
                            phase = %connection_error.lifecycle_error.failed_phase,
                            error = %connection_error.lifecycle_error.error_message,
                            "MCP connection failed with non-recoverable error - failing immediately"
                        );
                        return Err(connection_error);
                    }

                    // Recoverable error - check if we have retries left
                    attempt += 1;
                    if attempt >= max_attempts {
                        warn!(
                            server = %name,
                            attempts = attempt,
                            error = %connection_error.lifecycle_error.error_message,
                            "MCP connection failed after max retry attempts"
                        );
                        return Err(connection_error);
                    }

                    // Calculate exponential backoff delay
                    let delay_ms = base_delay_ms * 2u64.pow(attempt - 1);
                    warn!(
                        server = %name,
                        attempt = attempt,
                        max_attempts = max_attempts,
                        delay_ms = delay_ms,
                        error = %connection_error.lifecycle_error.error_message,
                        "MCP connection failed with recoverable error - retrying with backoff"
                    );

                    // Update reconnect attempts in state
                    {
                        let mut states = self.server_states.write().await;
                        if let Some(state) = states.get_mut(name) {
                            state.reconnect_attempts = attempt;
                            state.health = McpServerHealth::Reconnecting;
                        }
                    }

                    // Wait with exponential backoff before retry
                    tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms))
                        .await;
                }
            }
        }
    }

    /// Stop a specific server
    pub async fn stop_server(&self, name: &str) -> Result<(), SwellError> {
        // Get client and disconnect
        let client = {
            let clients = self.clients.read().await;
            clients.get(name).cloned()
        };

        if let Some(client) = client {
            client.disconnect().await;
        }

        // Remove client and update state
        {
            let mut clients = self.clients.write().await;
            clients.remove(name);
        }

        let mut states = self.server_states.write().await;
        if let Some(state) = states.get_mut(name) {
            state.health = McpServerHealth::Disconnected;
        }

        info!(server = %name, "MCP server stopped");
        Ok(())
    }

    /// Stop all servers
    pub async fn stop_all_servers(&self) {
        let names: Vec<String> = {
            let clients = self.clients.read().await;
            clients.keys().cloned().collect()
        };

        for name in names {
            self.stop_server(&name).await.ok();
        }
    }

    /// Check if a server is connected
    pub async fn is_server_connected(&self, name: &str) -> bool {
        let clients = self.clients.read().await;
        if let Some(client) = clients.get(name) {
            client.is_connected().await
        } else {
            false
        }
    }

    /// Attempt to reconnect a disconnected server
    pub async fn reconnect_server(&self, name: &str) -> Result<McpClient, SwellError> {
        // Check if max attempts reached
        let reconnect_attempts = {
            let states = self.server_states.read().await;
            let state = states.get(name).ok_or_else(|| {
                SwellError::ConfigError(format!("MCP server '{}' not found in state", name))
            })?;

            if state.reconnect_attempts >= self.reconnect_config.max_reconnect_attempts {
                return Err(SwellError::ToolExecutionFailed(format!(
                    "MCP server '{}' failed after {} reconnection attempts",
                    name, state.reconnect_attempts
                )));
            }

            state.reconnect_attempts
        };

        // Update state to reconnecting
        {
            let mut states = self.server_states.write().await;
            if let Some(state) = states.get_mut(name) {
                state.reconnect_attempts += 1;
                state.health = McpServerHealth::Reconnecting;
            }
        }

        info!(
            server = %name,
            attempt = reconnect_attempts + 1,
            max_attempts = self.reconnect_config.max_reconnect_attempts,
            "Attempting to reconnect MCP server"
        );

        // Stop existing client if any
        if let Some(client) = {
            let clients = self.clients.read().await;
            clients.get(name).cloned()
        } {
            client.disconnect().await;
            let mut clients = self.clients.write().await;
            clients.remove(name);
        }

        // Start server again (lazy start)
        self.start_server(name).await
    }

    /// Perform a health check on a server
    /// Returns true if server is healthy, false otherwise
    pub async fn health_check(&self, name: &str) -> bool {
        let client = {
            let clients = self.clients.read().await;
            clients.get(name).cloned()
        };

        match client {
            Some(client) if client.is_connected().await => {
                // Server is connected, try to list tools as health check
                match client.list_tools().await {
                    Ok(_) => {
                        self.update_health(name, McpServerHealth::Healthy).await;
                        true
                    }
                    Err(e) => {
                        warn!(server = %name, error = %e, "Health check failed - server unresponsive");
                        false
                    }
                }
            }
            _ => {
                // Server not connected
                self.update_health(name, McpServerHealth::Disconnected)
                    .await;
                false
            }
        }
    }

    /// Update health status for a server
    async fn update_health(&self, name: &str, health: McpServerHealth) {
        let mut states = self.server_states.write().await;
        if let Some(state) = states.get_mut(name) {
            if health == McpServerHealth::Healthy {
                state.last_connected_ms = Some(chrono::Utc::now().timestamp_millis() as u64);
                state.reconnect_attempts = 0;
            }
            state.health = health;
        }
    }

    /// Start all servers (non-lazy mode)
    /// Returns success even if some servers fail to start
    pub async fn start_all_servers(&self) -> Result<(), SwellError> {
        for server_config in &self.config.servers {
            if !self.is_server_connected(&server_config.name).await {
                self.start_server(&server_config.name).await?;
            }
        }
        Ok(())
    }

    /// Start all servers with degraded mode - daemons continues even if some servers fail.
    /// Failed servers are marked as Degraded or Failed instead of causing startup to abort.
    /// This allows the system to start with partial MCP functionality.
    pub async fn start_all_servers_degraded(&self) -> HashMap<String, McpServerHealth> {
        let mut results = HashMap::new();

        for server_config in &self.config.servers {
            if self.is_server_connected(&server_config.name).await {
                results.insert(server_config.name.clone(), McpServerHealth::Healthy);
                continue;
            }

            let health = match self.start_server(&server_config.name).await {
                Ok(_) => McpServerHealth::Healthy,
                Err(e) => {
                    error!(
                        server = %server_config.name,
                        error = %e,
                        "MCP server failed to start, marking as degraded"
                    );
                    // Mark as Degraded since this is the first attempt failure
                    // If reconnect attempts also fail, it will become Failed
                    McpServerHealth::Degraded
                }
            };

            results.insert(server_config.name.clone(), health);
        }

        results
    }

    /// Get all server names
    pub fn server_names(&self) -> Vec<&str> {
        self.config.server_names()
    }

    /// Get list of all tools from all servers (starting servers as needed)
    pub async fn list_all_tools(&self) -> Result<HashMap<String, Vec<McpToolInfo>>, SwellError> {
        let mut result = HashMap::new();

        for server_config in &self.config.servers {
            let client = match self.get_or_start_server(&server_config.name).await {
                Ok(c) => c,
                Err(e) => {
                    warn!(server = %server_config.name, error = %e, "Failed to start server");
                    continue;
                }
            };

            match client.list_tools().await {
                Ok(tools) => {
                    result.insert(server_config.name.clone(), tools);
                }
                Err(e) => {
                    warn!(server = %server_config.name, error = %e, "Failed to list tools");
                }
            }
        }

        Ok(result)
    }

    /// Get a tool wrapper from a specific server (starting server as needed)
    /// Note: Returns the tool info directly since McpToolWrapper constructor is private
    pub async fn get_tool(
        &self,
        server_name: &str,
        tool_name: &str,
    ) -> Result<McpToolInfo, SwellError> {
        let client = self.get_or_start_server(server_name).await?;

        // Get cached tools or refresh
        let tools = client.list_tools_deferred().await?;

        tools
            .into_iter()
            .find(|t| t.name == tool_name)
            .ok_or_else(|| {
                SwellError::ToolExecutionFailed(format!(
                    "MCP tool '{}' not found on server '{}'",
                    tool_name, server_name
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_server_config_to_command_string() {
        let config = McpServerConfig {
            name: "tree-sitter".to_string(),
            command: "python3".to_string(),
            args: vec!["-m".to_string(), "mcp_server_tree_sitter".to_string()],
            env: HashMap::new(),
        };

        assert_eq!(
            config.to_command_string(),
            "python3 -m mcp_server_tree_sitter"
        );
    }

    #[test]
    fn test_mcp_server_config_is_valid() {
        let valid = McpServerConfig {
            name: "test".to_string(),
            command: "python3".to_string(),
            args: vec!["-m".to_string(), "test".to_string()],
            env: HashMap::new(),
        };
        assert!(valid.is_valid());

        let invalid_name = McpServerConfig {
            name: "".to_string(),
            command: "python3".to_string(),
            args: vec![],
            env: HashMap::new(),
        };
        assert!(!invalid_name.is_valid());

        let invalid_command = McpServerConfig {
            name: "test".to_string(),
            command: "".to_string(),
            args: vec![],
            env: HashMap::new(),
        };
        assert!(!invalid_command.is_valid());
    }

    #[test]
    fn test_mcp_servers_config_load_from_str() {
        let json = r#"{
            "servers": [
                {
                    "name": "tree-sitter",
                    "command": "python3",
                    "args": ["-m", "mcp_server_tree_sitter"],
                    "env": {}
                },
                {
                    "name": "rust-analyzer",
                    "command": "npx",
                    "args": ["-y", "mcp-language-server"],
                    "env": {"RUST_ANALYZER_MODE": "debug"}
                }
            ]
        }"#;

        let config = McpServersConfig::load_from_str(json).unwrap();
        assert_eq!(config.servers.len(), 2);

        let tree_sitter = config.get_server("tree-sitter").unwrap();
        assert_eq!(tree_sitter.name, "tree-sitter");
        assert_eq!(tree_sitter.command, "python3");
        assert_eq!(tree_sitter.args, vec!["-m", "mcp_server_tree_sitter"]);

        let rust_analyser = config.get_server("rust-analyzer").unwrap();
        assert_eq!(rust_analyser.command, "npx");
        assert_eq!(
            rust_analyser.env.get("RUST_ANALYZER_MODE").unwrap(),
            "debug"
        );
    }

    #[test]
    fn test_mcp_servers_config_empty() {
        let json = "{}";
        let config = McpServersConfig::load_from_str(json).unwrap();
        assert!(config.servers.is_empty());
    }

    #[test]
    fn test_mcp_servers_config_server_names() {
        let config = McpServersConfig {
            servers: vec![
                McpServerConfig {
                    name: "server1".to_string(),
                    command: "cmd1".to_string(),
                    args: vec![],
                    env: HashMap::new(),
                },
                McpServerConfig {
                    name: "server2".to_string(),
                    command: "cmd2".to_string(),
                    args: vec![],
                    env: HashMap::new(),
                },
            ],
        };

        assert_eq!(config.server_names(), vec!["server1", "server2"]);
    }

    #[test]
    fn test_mcp_config_manager_new_from_str() {
        let json = r#"{
            "servers": [
                {
                    "name": "test-server",
                    "command": "echo",
                    "args": ["hello"],
                    "env": {}
                }
            ]
        }"#;

        let manager = McpConfigManager::new_from_str(json).unwrap();
        assert_eq!(manager.server_names(), vec!["test-server"]);
    }

    #[tokio::test]
    async fn test_mcp_config_manager_get_server_health_disconnected() {
        let json = r#"{
            "servers": [
                {
                    "name": "test-server",
                    "command": "echo",
                    "args": ["hello"],
                    "env": {}
                }
            ]
        }"#;

        let manager = McpConfigManager::new_from_str(json).unwrap();
        let health = manager.get_server_health("test-server").await;
        assert_eq!(health, McpServerHealth::Disconnected);
    }

    #[tokio::test]
    async fn test_mcp_config_manager_get_unknown_server_health() {
        let json = r#"{"servers": []}"#;
        let manager = McpConfigManager::new_from_str(json).unwrap();
        let health = manager.get_server_health("nonexistent").await;
        assert_eq!(health, McpServerHealth::Disconnected);
    }

    #[tokio::test]
    async fn test_mcp_config_manager_deferred_load() {
        let json = r#"{"servers": []}"#;
        let manager = McpConfigManager::new_from_str(json).unwrap();

        // Should start enabled
        assert!(manager.is_deferred_load_enabled().await);

        // Should be able to disable
        manager.set_deferred_load(false).await;
        assert!(!manager.is_deferred_load_enabled().await);
    }

    #[tokio::test]
    async fn test_mcp_config_manager_reconnect_config() {
        let json = r#"{"servers": []}"#;
        let manager = McpConfigManager::new_from_str(json).unwrap();

        let config = manager.reconnect_config();
        assert_eq!(config.max_reconnect_attempts, 3);
        assert_eq!(config.reconnect_delay_ms, 1000);
        assert!(config.auto_reconnect);
    }

    #[tokio::test]
    async fn test_mcp_config_manager_stop_all_servers() {
        let json = r#"{
            "servers": [
                {
                    "name": "test-server",
                    "command": "echo",
                    "args": ["hello"],
                    "env": {}
                }
            ]
        }"#;

        let manager = McpConfigManager::new_from_str(json).unwrap();
        // Should not panic even though no servers are running
        manager.stop_all_servers().await;
    }

    #[test]
    fn test_mcp_reconnect_config_default() {
        let config = McpReconnectConfig::default();
        assert_eq!(config.max_reconnect_attempts, 3);
        assert_eq!(config.reconnect_delay_ms, 1000);
        assert_eq!(config.health_check_interval_ms, 30000);
        assert!(config.auto_reconnect);
    }

    #[test]
    fn test_mcp_server_health_default() {
        let health = McpServerHealth::default();
        assert_eq!(health, McpServerHealth::Disconnected);
    }
}

// =============================================================================
// MCP Config Integration Tests
// =============================================================================

#[cfg(test)]
mod mcp_config_tests {
    use super::*;

    const EXAMPLE_CONFIG_JSON: &str = r#"{
        "servers": [
            {
                "name": "tree-sitter",
                "command": "python3",
                "args": ["-m", "mcp_server_tree_sitter"],
                "env": {}
            },
            {
                "name": "rust-analyzer",
                "command": "npx",
                "args": ["-y", "mcp-language-server", "--lsp", "rust-analyzer"],
                "env": {"RUST_BACKTRACE": "1"}
            }
        ]
    }"#;

    #[test]
    fn test_load_example_config() {
        let config = McpServersConfig::load_from_str(EXAMPLE_CONFIG_JSON).unwrap();
        assert_eq!(config.servers.len(), 2);

        let tree_sitter = config.get_server("tree-sitter").unwrap();
        assert_eq!(tree_sitter.command, "python3");
        assert_eq!(tree_sitter.args, vec!["-m", "mcp_server_tree_sitter"]);
        assert!(tree_sitter.env.is_empty());

        let rust_analyzer = config.get_server("rust-analyzer").unwrap();
        assert_eq!(rust_analyzer.command, "npx");
        assert_eq!(
            rust_analyzer.args,
            vec!["-y", "mcp-language-server", "--lsp", "rust-analyzer"]
        );
        assert_eq!(rust_analyzer.env.get("RUST_BACKTRACE").unwrap(), "1");
    }

    #[test]
    fn test_config_with_env_variables() {
        let json = r#"{
            "servers": [
                {
                    "name": "custom-server",
                    "command": "node",
                    "args": ["server.js"],
                    "env": {
                        "API_KEY": "secret123",
                        "LOG_LEVEL": "debug"
                    }
                }
            ]
        }"#;

        let config = McpServersConfig::load_from_str(json).unwrap();
        let server = config.get_server("custom-server").unwrap();

        assert_eq!(server.env.len(), 2);
        assert_eq!(server.env.get("API_KEY").unwrap(), "secret123");
        assert_eq!(server.env.get("LOG_LEVEL").unwrap(), "debug");
    }

    #[test]
    fn test_config_command_string_generation() {
        let json = r#"{
            "servers": [
                {
                    "name": "test",
                    "command": "python3",
                    "args": ["-m", "http.server", "8080"],
                    "env": {}
                }
            ]
        }"#;

        let config = McpServersConfig::load_from_str(json).unwrap();
        let server = config.get_server("test").unwrap();

        assert_eq!(server.to_command_string(), "python3 -m http.server 8080");
    }

    #[test]
    fn test_config_with_no_args() {
        let json = r#"{
            "servers": [
                {
                    "name": "simple",
                    "command": "echo",
                    "args": [],
                    "env": {}
                }
            ]
        }"#;

        let config = McpServersConfig::load_from_str(json).unwrap();
        let server = config.get_server("simple").unwrap();

        assert_eq!(server.to_command_string(), "echo");
    }

    #[test]
    fn test_get_nonexistent_server() {
        let json = r#"{"servers": []}"#;
        let config = McpServersConfig::load_from_str(json).unwrap();
        assert!(config.get_server("nonexistent").is_none());
    }

    #[tokio::test]
    async fn test_manager_with_reconnect_config() {
        let json = r#"{"servers": []}"#;
        let manager = McpConfigManager::new_from_str(json).unwrap();

        let custom_config = McpReconnectConfig {
            max_reconnect_attempts: 5,
            reconnect_delay_ms: 2000,
            health_check_interval_ms: 60000,
            auto_reconnect: true,
        };

        let manager = manager.with_reconnect_config(custom_config);
        let config = manager.reconnect_config();

        assert_eq!(config.max_reconnect_attempts, 5);
        assert_eq!(config.reconnect_delay_ms, 2000);
        assert_eq!(config.health_check_interval_ms, 60000);
    }

    #[tokio::test]
    async fn test_health_check_for_unknown_server() {
        let json = r#"{
            "servers": [
                {
                    "name": "unknown",
                    "command": "nonexistent",
                    "args": [],
                    "env": {}
                }
            ]
        }"#;

        let manager = McpConfigManager::new_from_str(json).unwrap();

        // Should return false for unknown server (not started)
        let is_healthy = manager.health_check("unknown").await;
        assert!(!is_healthy);
    }

    #[tokio::test]
    async fn test_get_all_health() {
        let json = r#"{
            "servers": [
                {"name": "server1", "command": "echo", "args": [], "env": {}},
                {"name": "server2", "command": "echo", "args": [], "env": {}}
            ]
        }"#;

        let manager = McpConfigManager::new_from_str(json).unwrap();
        let all_health = manager.get_all_health().await;

        assert_eq!(all_health.len(), 2);
        assert_eq!(
            all_health.get("server1"),
            Some(&McpServerHealth::Disconnected)
        );
        assert_eq!(
            all_health.get("server2"),
            Some(&McpServerHealth::Disconnected)
        );
    }

    #[test]
    fn test_config_with_complex_args() {
        let json = r#"{
            "servers": [
                {
                    "name": "complex",
                    "command": "docker",
                    "args": ["run", "--rm", "-v", "/host:/container", "image"],
                    "env": {}
                }
            ]
        }"#;

        let config = McpServersConfig::load_from_str(json).unwrap();
        let server = config.get_server("complex").unwrap();

        let cmd_str = server.to_command_string();
        assert_eq!(cmd_str, "docker run --rm -v /host:/container image");
    }
}
