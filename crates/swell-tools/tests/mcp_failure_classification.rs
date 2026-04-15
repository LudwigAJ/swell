//! MCP Failure Classification Tests
//!
//! These tests verify that MCP connection failures are properly classified into
//! recoverable (timeout, connection refused → retry with backoff) and
//! non-recoverable (binary not found, protocol mismatch → immediate fail).
//!
//! Reference: VAL-MCP-003

#[cfg(test)]
mod mcp_failure_classification_tests {

    use swell_tools::mcp::{McpConnectionError, McpFailureClass, McpLifecyclePhase};

    /// Test that McpFailureClass::Recoverable is correctly identified
    #[test]
    fn test_failure_class_recoverable() {
        let class = McpFailureClass::Recoverable;
        assert!(class.is_recoverable());
        assert!(!class.is_non_recoverable());
    }

    /// Test that McpFailureClass::NonRecoverable is correctly identified
    #[test]
    fn test_failure_class_non_recoverable() {
        let class = McpFailureClass::NonRecoverable;
        assert!(!class.is_recoverable());
        assert!(class.is_non_recoverable());
    }

    /// Test that binary not found is classified as non-recoverable
    #[test]
    fn test_binary_not_found_is_non_recoverable() {
        // "No such file or directory" indicates binary not found
        let class = McpConnectionError::classify_error(
            McpLifecyclePhase::SpawnConnect,
            "No such file or directory: nonexistent_command",
        );
        assert_eq!(
            class,
            McpFailureClass::NonRecoverable,
            "Binary not found should be non-recoverable"
        );
    }

    /// Test that "not found" error is classified as non-recoverable
    #[test]
    fn test_not_found_error_is_non_recoverable() {
        let class = McpConnectionError::classify_error(
            McpLifecyclePhase::SpawnConnect,
            "Command not found: invalid_binary",
        );
        assert_eq!(
            class,
            McpFailureClass::NonRecoverable,
            "'not found' error should be non-recoverable"
        );
    }

    /// Test that "executable" error is classified as non-recoverable
    #[test]
    fn test_executable_error_is_non_recoverable() {
        let class = McpConnectionError::classify_error(
            McpLifecyclePhase::SpawnConnect,
            "Cannot find executable: some_binary",
        );
        assert_eq!(
            class,
            McpFailureClass::NonRecoverable,
            "'executable' error should be non-recoverable"
        );
    }

    /// Test that "enoent" error is classified as non-recoverable
    #[test]
    fn test_enoent_error_is_non_recoverable() {
        let class = McpConnectionError::classify_error(
            McpLifecyclePhase::SpawnConnect,
            "ENOENT: no such file or directory",
        );
        assert_eq!(
            class,
            McpFailureClass::NonRecoverable,
            "ENOENT error should be non-recoverable"
        );
    }

    /// Test that timeout is classified as recoverable
    #[test]
    fn test_timeout_is_recoverable() {
        let class = McpConnectionError::classify_error(
            McpLifecyclePhase::SpawnConnect,
            "Connection timed out after 30 seconds",
        );
        assert_eq!(
            class,
            McpFailureClass::Recoverable,
            "Timeout should be recoverable"
        );
    }

    /// Test that connection refused is classified as recoverable
    #[test]
    fn test_connection_refused_is_recoverable() {
        let class = McpConnectionError::classify_error(
            McpLifecyclePhase::SpawnConnect,
            "Connection refused: ECONNREFUSED",
        );
        assert_eq!(
            class,
            McpFailureClass::Recoverable,
            "Connection refused should be recoverable"
        );
    }

    /// Test that connection reset is classified as recoverable
    #[test]
    fn test_connection_reset_is_recoverable() {
        let class = McpConnectionError::classify_error(
            McpLifecyclePhase::SpawnConnect,
            "Connection reset by peer",
        );
        assert_eq!(
            class,
            McpFailureClass::Recoverable,
            "Connection reset should be recoverable"
        );
    }

    /// Test that broken pipe is classified as recoverable
    #[test]
    fn test_broken_pipe_is_recoverable() {
        let class = McpConnectionError::classify_error(
            McpLifecyclePhase::SpawnConnect,
            "Broken pipe: EPIPE",
        );
        assert_eq!(
            class,
            McpFailureClass::Recoverable,
            "Broken pipe should be recoverable"
        );
    }

    /// Test that protocol mismatch is classified as non-recoverable
    #[test]
    fn test_protocol_mismatch_is_non_recoverable() {
        let class = McpConnectionError::classify_error(
            McpLifecyclePhase::InitializeHandshake,
            "Protocol version mismatch: expected 2024-11-05, got 2024-10-01",
        );
        assert_eq!(
            class,
            McpFailureClass::NonRecoverable,
            "Protocol version mismatch should be non-recoverable"
        );
    }

    /// Test that incompatible protocol is classified as non-recoverable
    #[test]
    fn test_incompatible_protocol_is_non_recoverable() {
        let class = McpConnectionError::classify_error(
            McpLifecyclePhase::InitializeHandshake,
            "Incompatible protocol version",
        );
        assert_eq!(
            class,
            McpFailureClass::NonRecoverable,
            "Incompatible protocol should be non-recoverable"
        );
    }

    /// Test that timeout during initialization handshake is recoverable
    #[test]
    fn test_init_timeout_is_recoverable() {
        let class = McpConnectionError::classify_error(
            McpLifecyclePhase::InitializeHandshake,
            "Handshake timed out after 10 seconds",
        );
        assert_eq!(
            class,
            McpFailureClass::Recoverable,
            "Timeout during initialization should be recoverable"
        );
    }

    /// Test that empty command is classified as non-recoverable (ConfigLoad phase)
    #[test]
    fn test_empty_command_is_non_recoverable() {
        let class = McpConnectionError::classify_error(
            McpLifecyclePhase::ConfigLoad,
            "MCP server command is empty",
        );
        assert_eq!(
            class,
            McpFailureClass::NonRecoverable,
            "Empty command should be non-recoverable"
        );
    }

    /// Test that invalid command is classified as non-recoverable (ConfigLoad phase)
    #[test]
    fn test_invalid_command_is_non_recoverable() {
        let class = McpConnectionError::classify_error(
            McpLifecyclePhase::ConfigLoad,
            "Invalid command format",
        );
        assert_eq!(
            class,
            McpFailureClass::NonRecoverable,
            "Invalid command should be non-recoverable"
        );
    }

    /// Test that tool discovery timeout is classified as recoverable
    #[test]
    fn test_tool_discovery_timeout_is_recoverable() {
        let class = McpConnectionError::classify_error(
            McpLifecyclePhase::ToolDiscovery,
            "Tool discovery timed out",
        );
        assert_eq!(
            class,
            McpFailureClass::Recoverable,
            "Tool discovery timeout should be recoverable"
        );
    }

    /// Test McpConnectionError::is_recoverable returns correct value
    #[test]
    fn test_mcp_connection_error_is_recoverable() {
        use swell_tools::McpLifecycleError;

        // Create a recoverable error
        let lifecycle_error =
            McpLifecycleError::new(McpLifecyclePhase::SpawnConnect, "Connection refused");
        let connection_error = McpConnectionError::from_lifecycle_error(lifecycle_error);
        assert!(
            connection_error.is_recoverable(),
            "Connection refused should be recoverable"
        );

        // Create a non-recoverable error
        let lifecycle_error =
            McpLifecycleError::new(McpLifecyclePhase::SpawnConnect, "No such file or directory");
        let connection_error = McpConnectionError::from_lifecycle_error(lifecycle_error);
        assert!(
            connection_error.is_non_recoverable(),
            "Binary not found should be non-recoverable"
        );
    }

    /// Test McpConnectionError::is_non_recoverable returns correct value
    #[test]
    fn test_mcp_connection_error_is_non_recoverable() {
        use swell_tools::McpLifecycleError;

        // Protocol version mismatch happens during InitializeHandshake phase
        let lifecycle_error = McpLifecycleError::new(
            McpLifecyclePhase::InitializeHandshake,
            "Protocol version mismatch",
        );
        let connection_error = McpConnectionError::from_lifecycle_error(lifecycle_error);
        assert!(
            connection_error.is_non_recoverable(),
            "Protocol mismatch should be non-recoverable"
        );
    }

    /// Test McpConnectionError display includes classification
    #[test]
    fn test_mcp_connection_error_display() {
        use swell_tools::McpLifecycleError;

        let lifecycle_error =
            McpLifecycleError::new(McpLifecyclePhase::SpawnConnect, "Connection refused");
        let connection_error = McpConnectionError::from_lifecycle_error(lifecycle_error);

        let display = format!("{}", connection_error);
        assert!(
            display.contains("recoverable"),
            "Display should include 'recoverable' classification"
        );

        let lifecycle_error =
            McpLifecycleError::new(McpLifecyclePhase::SpawnConnect, "Binary not found");
        let connection_error = McpConnectionError::from_lifecycle_error(lifecycle_error);

        let display = format!("{}", connection_error);
        assert!(
            display.contains("non-recoverable"),
            "Display should include 'non-recoverable' classification"
        );
    }

    /// Test that econnrefused is classified as recoverable
    #[test]
    fn test_econnrefused_is_recoverable() {
        let class = McpConnectionError::classify_error(
            McpLifecyclePhase::SpawnConnect,
            "ECONNREFUSED: connection refused",
        );
        assert_eq!(
            class,
            McpFailureClass::Recoverable,
            "ECONNREFUSED should be recoverable"
        );
    }

    /// Test that etimedout is classified as recoverable
    #[test]
    fn test_etimedout_is_recoverable() {
        let class = McpConnectionError::classify_error(
            McpLifecyclePhase::SpawnConnect,
            "ETIMEDOUT: connection timed out",
        );
        assert_eq!(
            class,
            McpFailureClass::Recoverable,
            "ETIMEDOUT should be recoverable"
        );
    }
}

#[cfg(test)]
mod mcp_failure_classification_integration_tests {

    use swell_tools::mcp_config::{McpConfigManager, McpReconnectConfig, McpServerHealth};

    /// Test that non-recoverable errors fail immediately without retry
    /// Binary not found should fail immediately without waiting for retries
    #[tokio::test]
    async fn test_non_recoverable_error_fails_immediately() {
        // Create config with a server that uses a non-existent binary
        let json = serde_json::json!({
            "servers": [
                {
                    "name": "nonexistent-server",
                    "command": "definitely_not_a_real_command_xyz_12345",
                    "args": [],
                    "env": {}
                }
            ]
        })
        .to_string();

        let reconnect_config = McpReconnectConfig {
            max_reconnect_attempts: 3,
            reconnect_delay_ms: 1000, // Would be used for retries if error was recoverable
            health_check_interval_ms: 1000,
            auto_reconnect: true,
        };

        let manager = McpConfigManager::new_from_str(&json)
            .unwrap()
            .with_reconnect_config(reconnect_config);

        // Start server - should fail immediately due to non-recoverable error
        let start = std::time::Instant::now();
        let result = manager.start_server("nonexistent-server").await;
        let elapsed = start.elapsed();

        // Should have failed
        assert!(result.is_err(), "Server should have failed to start");

        // Should have failed quickly (not waiting for retries)
        // If it waited for retries, it would take at least 1 second (reconnect_delay_ms * attempts)
        // With immediate failure, it should be under 500ms
        assert!(
            elapsed.as_millis() < 500,
            "Non-recoverable error should fail immediately, took {}ms",
            elapsed.as_millis()
        );

        // Health should be Failed (not just Disconnected or Reconnecting)
        let health = manager.get_server_health("nonexistent-server").await;
        assert_eq!(
            health,
            McpServerHealth::Failed,
            "Non-recoverable error should result in Failed health status"
        );
    }

    /// Test that degraded startup completes even with failing servers
    #[tokio::test]
    async fn test_degraded_startup_with_classification() {
        let json = serde_json::json!({
            "servers": [
                {
                    "name": "valid-server",
                    "command": "echo",
                    "args": ["hello"],
                    "env": {}
                },
                {
                    "name": "invalid-server",
                    "command": "nonexistent_binary_xyz",
                    "args": [],
                    "env": {}
                }
            ]
        })
        .to_string();

        let manager = McpConfigManager::new_from_str(&json).unwrap();

        // start_all_servers_degraded should not return an error even with failing server
        let results = manager.start_all_servers_degraded().await;

        // Results should contain both servers
        assert!(results.contains_key("valid-server"));
        assert!(results.contains_key("invalid-server"));

        // Invalid server should be degraded or failed (not healthy)
        let invalid_health = results.get("invalid-server").unwrap();
        assert!(
            *invalid_health != McpServerHealth::Healthy,
            "Invalid server should not be Healthy, got {:?}",
            invalid_health
        );
    }
}
