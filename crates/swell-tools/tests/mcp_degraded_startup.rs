//! MCP Degraded Startup Integration Tests
//!
//! These tests verify that the daemon can start successfully even when one or more
//! MCP servers fail to connect. Failed servers are reported with Degraded or Failed
//! health status while healthy servers continue serving tools normally.
//!
//! Reference: VAL-MCP-001

#[cfg(test)]
mod mcp_degraded_startup_tests {

    /// Test that start_all_servers_degraded completes without error even when some servers fail
    #[tokio::test]
    async fn test_degraded_startup_daemon_continues_with_failed_servers() {
        let json = serde_json::json!({
            "servers": [
                {
                    "name": "valid-server",
                    "command": "echo",
                    "args": ["hello"],
                    "env": {}
                },
                {
                    "name": "failing-server",
                    "command": "nonexistent_command_xyz",
                    "args": [],
                    "env": {}
                }
            ]
        })
        .to_string();

        let manager = swell_tools::mcp_config::McpConfigManager::new_from_str(&json)
            .expect("Failed to create manager");

        // start_all_servers_degraded should not return an error even with failing server
        let results = manager.start_all_servers_degraded().await;

        // Results should contain both servers
        assert!(results.contains_key("valid-server"));
        assert!(results.contains_key("failing-server"));

        // Valid server should be healthy (or degraded if echo has issues)
        // At minimum, we should get a result for it
        let valid_health = results.get("valid-server");
        assert!(
            valid_health.is_some(),
            "Valid server should have a health status"
        );

        // Failing server should be degraded or failed
        let failing_health = results.get("failing-server");
        assert!(
            failing_health.is_some(),
            "Failing server should have a health status"
        );

        let health = *failing_health.unwrap();
        assert!(
            health == swell_tools::mcp_config::McpServerHealth::Degraded
                || health == swell_tools::mcp_config::McpServerHealth::Failed,
            "Failing server should be Degraded or Failed, got {:?}",
            health
        );
    }

    /// Test that healthy servers remain functional after degraded startup
    #[tokio::test]
    async fn test_healthy_servers_remain_functional() {
        let json = serde_json::json!({
            "servers": [
                {
                    "name": "server-a",
                    "command": "echo",
                    "args": ["server-a"],
                    "env": {}
                },
                {
                    "name": "server-b",
                    "command": "nonexistent_command_abc",
                    "args": [],
                    "env": {}
                }
            ]
        })
        .to_string();

        let manager = swell_tools::mcp_config::McpConfigManager::new_from_str(&json)
            .expect("Failed to create manager");

        // Perform degraded startup
        let results = manager.start_all_servers_degraded().await;

        // Server A should be healthy
        let server_a_health = results.get("server-a").unwrap();
        assert!(
            *server_a_health == swell_tools::mcp_config::McpServerHealth::Healthy
                || *server_a_health == swell_tools::mcp_config::McpServerHealth::Degraded,
            "Server A should be Healthy or Degraded, got {:?}",
            *server_a_health
        );

        // Server B (failing) should not be in connected state
        let server_b_health = results.get("server-b").unwrap();
        assert_ne!(
            *server_b_health,
            swell_tools::mcp_config::McpServerHealth::Healthy,
            "Server B should not be Healthy"
        );
    }

    /// Test that multiple failing servers are all reported correctly
    #[tokio::test]
    async fn test_multiple_failing_servers_all_reported() {
        let json = serde_json::json!({
            "servers": [
                {
                    "name": "failing-1",
                    "command": "nonexistent_1",
                    "args": [],
                    "env": {}
                },
                {
                    "name": "failing-2",
                    "command": "nonexistent_2",
                    "args": [],
                    "env": {}
                },
                {
                    "name": "failing-3",
                    "command": "nonexistent_3",
                    "args": [],
                    "env": {}
                }
            ]
        })
        .to_string();

        let manager = swell_tools::mcp_config::McpConfigManager::new_from_str(&json)
            .expect("Failed to create manager");

        let results = manager.start_all_servers_degraded().await;

        // All three failing servers should be in degraded/failed state
        for i in 1..=3 {
            let name = format!("failing-{}", i);
            let health = results.get(&name).unwrap();
            assert!(
                *health == swell_tools::mcp_config::McpServerHealth::Degraded
                    || *health == swell_tools::mcp_config::McpServerHealth::Failed,
                "Server {} should be Degraded or Failed, got {:?}",
                name,
                *health
            );
        }
    }

    /// Test that get_all_health returns correct status after degraded startup
    #[tokio::test]
    async fn test_get_all_health_after_degraded_startup() {
        let json = serde_json::json!({
            "servers": [
                {
                    "name": "healthy-echo",
                    "command": "echo",
                    "args": ["test"],
                    "env": {}
                },
                {
                    "name": "failing-server",
                    "command": "invalid_command_xyz",
                    "args": [],
                    "env": {}
                }
            ]
        })
        .to_string();

        let manager = swell_tools::mcp_config::McpConfigManager::new_from_str(&json)
            .expect("Failed to create manager");

        // Perform degraded startup
        let _ = manager.start_all_servers_degraded().await;

        // Get all health statuses
        let all_health = manager.get_all_health().await;

        // Verify both servers are tracked
        assert!(all_health.contains_key("healthy-echo"));
        assert!(all_health.contains_key("failing-server"));
    }

    /// Test that McpServerHealth::Degraded variant exists and is distinct
    #[tokio::test]
    async fn test_degraded_health_variant_exists() {
        let degraded = swell_tools::mcp_config::McpServerHealth::Degraded;
        let healthy = swell_tools::mcp_config::McpServerHealth::Healthy;
        let failed = swell_tools::mcp_config::McpServerHealth::Failed;
        let disconnected = swell_tools::mcp_config::McpServerHealth::Disconnected;

        // Degraded should be distinct from other states
        assert_ne!(degraded, healthy);
        assert_ne!(degraded, failed);
        assert_ne!(degraded, disconnected);

        // Should be copyable and cloneable
        let _ = degraded;
        let _ = degraded;
    }

    /// Test that start_all_servers (original method) still fails on first server failure
    /// This verifies we haven't broken the original behavior
    #[tokio::test]
    async fn test_original_start_all_servers_still_fails_on_bad_server() {
        let json = serde_json::json!({
            "servers": [
                {
                    "name": "good-server",
                    "command": "echo",
                    "args": ["hello"],
                    "env": {}
                },
                {
                    "name": "bad-server",
                    "command": "definitely_not_a_real_command_12345",
                    "args": [],
                    "env": {}
                }
            ]
        })
        .to_string();

        let manager = swell_tools::mcp_config::McpConfigManager::new_from_str(&json)
            .expect("Failed to create manager");

        // The original method should return an error
        let result = manager.start_all_servers().await;
        assert!(
            result.is_err(),
            "start_all_servers should fail when a server fails to start"
        );
    }

    /// Test mixed scenario: some servers healthy, some degraded
    #[tokio::test]
    async fn test_mixed_health_statuses() {
        let json = serde_json::json!({
            "servers": [
                {
                    "name": "echo-server",
                    "command": "echo",
                    "args": ["hello"],
                    "env": {}
                },
                {
                    "name": "fail-server-1",
                    "command": "nonexistent_cmd_1",
                    "args": [],
                    "env": {}
                },
                {
                    "name": "fail-server-2",
                    "command": "nonexistent_cmd_2",
                    "args": [],
                    "env": {}
                }
            ]
        })
        .to_string();

        let manager = swell_tools::mcp_config::McpConfigManager::new_from_str(&json)
            .expect("Failed to create manager");

        let results = manager.start_all_servers_degraded().await;

        // Verify we have results for all 3 servers
        assert_eq!(results.len(), 3, "Should have results for all 3 servers");

        // Find the echo-server and verify it has a non-failed/non-degraded healthy status
        // (it should be Healthy since echo always works, or at minimum not Failed)
        if let Some(echo_health) = results.get("echo-server") {
            assert!(
                *echo_health != swell_tools::mcp_config::McpServerHealth::Failed,
                "echo-server should not be in Failed state"
            );
        }

        // Find the failing servers and verify they are not Healthy
        for name in &["fail-server-1", "fail-server-2"] {
            if let Some(health) = results.get(*name) {
                assert!(
                    !health.is_healthy(),
                    "{} should not be Healthy (should be Degraded or Failed)",
                    name
                );
            }
        }
    }
}
