//! Kill switch integration for the orchestrator.
//!
//! This module provides the integration between the core KillSwitch system
//! and the orchestrator's execution loop. It adds:
//! - Kill switch checks at each orchestrator tick
//! - Tool dispatch interception for restriction enforcement
//! - External trigger support via file sentinel
//!
//! # Kill Switch Levels
//!
//! The kill switch system provides 4 ordered levels with escalating severity:
//! - **Throttle (1)**: Rate-limit operations, allows all other operations
//! - **ScopeBlock (2)**: Block file operations on specific paths, allows network and execution
//! - **NetworkKill (3)**: Block all network operations, allows file ops and execution
//! - **FullStop (4)**: Halts all agent execution immediately
//!
//! Higher levels include all restrictions of lower levels:
//! - FullStop blocks everything
//! - NetworkKill blocks network + ScopeBlock restrictions + Throttle restrictions
//! - ScopeBlock blocks file ops on specific paths + Throttle restrictions
//! - Throttle rate-limits operations

use std::sync::Arc;
use swell_core::kill_switch::{
    KillLevel, KillSwitchError, KillSwitchGuard, KillSwitchState, ScopeBlock, ThrottleConfig,
};

/// Result type for kill switch checks
pub type KillSwitchResult = Result<(), KillSwitchError>;

/// Orchestrator kill switch manager that wraps the core KillSwitchGuard
/// and provides orchestrator-specific functionality.
#[derive(Clone)]
pub struct OrchestratorKillSwitch {
    /// The underlying kill switch guard
    guard: Arc<KillSwitchGuard>,
}

impl std::fmt::Debug for OrchestratorKillSwitch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrchestratorKillSwitch")
            .finish_non_exhaustive()
    }
}

impl OrchestratorKillSwitch {
    /// Create a new orchestrator kill switch with default settings.
    pub fn new() -> Self {
        Self {
            guard: Arc::new(KillSwitchGuard::new()),
        }
    }

    /// Create with a custom kill switch guard.
    pub fn with_guard(guard: KillSwitchGuard) -> Self {
        Self {
            guard: Arc::new(guard),
        }
    }

    /// Add file-based sentinel verification.
    ///
    /// The sentinel file contains a kill level string (e.g., "full_stop", "throttle").
    /// When the file exists and contains a valid level, the kill switch is triggered.
    pub fn with_file_sentinel(self, path: impl Into<std::path::PathBuf>) -> Self {
        let guard = KillSwitchGuard::new().with_file_verifier(path);
        Self {
            guard: Arc::new(guard),
        }
    }

    /// Add environment variable verification.
    pub fn with_env_verifier(self, var_name: impl Into<String>) -> Self {
        let guard = KillSwitchGuard::new().with_env_verifier(var_name);
        Self {
            guard: Arc::new(guard),
        }
    }

    /// Get the underlying guard for direct access.
    pub fn guard(&self) -> Arc<KillSwitchGuard> {
        self.guard.clone()
    }

    /// Check if FullStop is active - halts all execution.
    ///
    /// This should be called at the start of each orchestrator tick
    /// and before any tool dispatch.
    pub async fn check_fullstop(&self) -> KillSwitchResult {
        self.guard.verify_external().await;
        let state = self.guard.state().await;

        if !state.active {
            return Ok(());
        }

        if let Some(level) = state.level {
            if level == KillLevel::FullStop {
                return Err(KillSwitchError::FullStop(state.reason));
            }
        }

        Ok(())
    }

    /// Check if network operations are allowed.
    ///
    /// Returns Err if NetworkKill or FullStop is active.
    pub async fn check_network(&self) -> KillSwitchResult {
        self.guard.verify_external().await;
        self.guard.check_network().await
    }

    /// Check if file operations on a path are allowed.
    ///
    /// Returns Err if ScopeBlock or FullStop is active and path is blocked.
    pub async fn check_path(&self, path: &str) -> KillSwitchResult {
        self.guard.verify_external().await;
        self.guard.check_path(path).await
    }

    /// Check throttle status and get delay if throttled.
    ///
    /// Returns Err with delay info if Throttle is active.
    pub async fn check_throttle(&self) -> KillSwitchResult {
        self.guard.verify_external().await;
        self.guard.check_throttle().await
    }

    /// Full check for tool dispatch - combines all restrictions.
    ///
    /// For a tool dispatch, we need to check:
    /// 1. FullStop blocks everything
    /// 2. NetworkKill blocks network-dependent tools
    /// 3. ScopeBlock blocks file operations on specific paths
    /// 4. Throttle adds delay between operations
    ///
    /// Returns Err with appropriate error if any restriction is active.
    pub async fn check_tool_dispatch(
        &self,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> KillSwitchResult {
        self.guard.verify_external().await;
        let state = self.guard.state().await;

        if !state.active {
            return Ok(());
        }

        let level = state.level.ok_or(KillSwitchError::Unknown)?;

        // FullStop blocks everything
        if level == KillLevel::FullStop {
            return Err(KillSwitchError::FullStop(state.reason));
        }

        // NetworkKill blocks network operations
        if level == KillLevel::NetworkKill && is_network_tool(tool_name) {
            return Err(KillSwitchError::NetworkKilled(state.reason));
        }

        // ScopeBlock blocks file operations on specific paths
        if (level == KillLevel::ScopeBlock || level == KillLevel::NetworkKill || level == KillLevel::FullStop)
            && is_file_tool(tool_name) {
            if let Some(path) = extract_path_argument(tool_name, arguments) {
                self.guard.check_path(&path).await?;
            }
        }

        // Throttle returns delay information
        if level == KillLevel::Throttle {
            return self.guard.check_throttle().await;
        }

        Ok(())
    }

    /// Manually trigger the kill switch at a specific level.
    pub async fn trigger(
        &self,
        level: KillLevel,
        reason: impl Into<String>,
        trigger: impl Into<String>,
    ) {
        self.guard.trigger(level, reason, trigger).await;
    }

    /// Reset the kill switch.
    pub async fn reset(&self) {
        self.guard.reset().await;
    }

    /// Get current state.
    pub async fn state(&self) -> KillSwitchState {
        self.guard.state().await
    }

    /// Set scope block configuration.
    pub async fn set_scope_block(&self, config: ScopeBlock) {
        self.guard.set_scope_block(config).await;
    }

    /// Set throttle configuration.
    pub async fn set_throttle(&self, config: ThrottleConfig) {
        self.guard.set_throttle(config).await;
    }

    /// Check if a specific level is active.
    ///
    /// Returns true if the current active level is >= the specified level
    /// (i.e., the restriction is in effect).
    pub async fn is_level_active(&self, level: KillLevel) -> bool {
        let state = self.guard.state().await;
        state.active && state.level.map(|l| l.severity()) >= Some(level.severity())
    }

    /// Get the current active level if any.
    pub async fn active_level(&self) -> Option<KillLevel> {
        let state = self.guard.state().await;
        state.level
    }

    /// Verify external sources and update state.
    pub async fn verify_external(&self) {
        self.guard.verify_external().await;
    }
}

impl Default for OrchestratorKillSwitch {
    fn default() -> Self {
        Self::new()
    }
}

/// Determine if a tool is network-dependent based on its name.
fn is_network_tool(tool_name: &str) -> bool {
    matches!(
        tool_name.to_lowercase().as_str(),
        "http_request"
            | "fetch"
            | "curl"
            | "wget"
            | "web_search"
            | "fetch_page"
            | "domain_search"
            | "send_request"
            | "api_call"
    )
}

/// Determine if a tool operates on files based on its name.
fn is_file_tool(tool_name: &str) -> bool {
    matches!(
        tool_name.to_lowercase().as_str(),
        "file_read"
            | "file_write"
            | "file_edit"
            | "read_file"
            | "write_file"
            | "edit_file"
            | "read"
            | "write"
            | "edit"
    )
}

/// Extract the path argument from tool arguments if present.
fn extract_path_argument(_tool_name: &str, arguments: &serde_json::Value) -> Option<String> {
    // Common path argument names
    let path_keys = ["path", "file", "file_path", "target", "destination", "src", "source"];

    // Try to find path in arguments
    if let Some(obj) = arguments.as_object() {
        for key in &path_keys {
            if let Some(value) = obj.get(*key) {
                if let Some(path_str) = value.as_str() {
                    return Some(path_str.to_string());
                }
            }
        }
    }

    None
}

/// Check if a tool name suggests it might operate on files.
pub fn is_potentially_file_tool(tool_name: &str) -> bool {
    let name = tool_name.to_lowercase();
    name.contains("file")
        || name.contains("read")
        || name.contains("write")
        || name.contains("edit")
        || name.contains("path")
}

/// Check if a tool name suggests it might be network-dependent.
pub fn is_potentially_network_tool(tool_name: &str) -> bool {
    let name = tool_name.to_lowercase();
    name.contains("http")
        || name.contains("fetch")
        || name.contains("curl")
        || name.contains("web")
        || name.contains("api")
        || name.contains("network")
        || name.contains("request")
        || name.contains("send")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_kill_switch_new_is_inactive() {
        let ks = OrchestratorKillSwitch::new();
        let state = ks.state().await;
        assert!(!state.active);
        assert!(state.level.is_none());
    }

    #[tokio::test]
    async fn test_trigger_fullstop_blocks_everything() {
        let ks = OrchestratorKillSwitch::new();
        ks.trigger(KillLevel::FullStop, "test", "test").await;

        let result = ks.check_fullstop().await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), KillSwitchError::FullStop(_)));
    }

    #[tokio::test]
    async fn test_trigger_throttle_allows_execution() {
        let ks = OrchestratorKillSwitch::new();
        ks.trigger(KillLevel::Throttle, "test", "test").await;

        // Throttle should not block fullstop check
        let result = ks.check_fullstop().await;
        assert!(result.is_ok());

        // But throttle should return delay info
        let result = ks.check_throttle().await;
        assert!(result.is_err());
        if let Err(KillSwitchError::ThrottledWithDelay { delay_ms, .. }) = result {
            assert_eq!(delay_ms, 5000); // Default delay
        } else {
            panic!("Expected ThrottledWithDelay");
        }
    }

    #[tokio::test]
    async fn test_network_kill_blocks_network_tools() {
        let ks = OrchestratorKillSwitch::new();
        ks.trigger(KillLevel::NetworkKill, "test", "test").await;

        // NetworkKill should block network tools
        let result = ks
            .check_tool_dispatch("http_request", &serde_json::json!({}))
            .await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), KillSwitchError::NetworkKilled(_)));

        // But should allow file tools
        let result = ks
            .check_tool_dispatch(
                "file_read",
                &serde_json::json!({"path": "/tmp/test.txt"}),
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_scope_block_blocks_specific_paths() {
        let ks = OrchestratorKillSwitch::new();
        ks.set_scope_block(ScopeBlock {
            blocked_paths: vec!["/secret".to_string()],
            blocked_patterns: vec![],
        })
        .await;
        ks.trigger(KillLevel::ScopeBlock, "test", "test")
            .await;

        // Should block access to /secret
        let result = ks.check_path("/secret/file.txt").await;
        assert!(result.is_err());

        // But allow other paths
        let result = ks.check_path("/allowed/file.txt").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_level_escalation() {
        let ks = OrchestratorKillSwitch::new();

        // Initially no level active
        assert!(!ks.is_level_active(KillLevel::Throttle).await);
        assert!(!ks.is_level_active(KillLevel::ScopeBlock).await);
        assert!(!ks.is_level_active(KillLevel::NetworkKill).await);
        assert!(!ks.is_level_active(KillLevel::FullStop).await);

        // Trigger Throttle
        ks.trigger(KillLevel::Throttle, "test", "test")
            .await;
        assert!(ks.is_level_active(KillLevel::Throttle).await);
        assert!(!ks.is_level_active(KillLevel::ScopeBlock).await); // Not yet
        assert!(!ks.is_level_active(KillLevel::NetworkKill).await);
        assert!(!ks.is_level_active(KillLevel::FullStop).await);

        // Escalate to ScopeBlock
        ks.trigger(KillLevel::ScopeBlock, "test", "test")
            .await;
        assert!(ks.is_level_active(KillLevel::Throttle).await);
        assert!(ks.is_level_active(KillLevel::ScopeBlock).await);
        assert!(!ks.is_level_active(KillLevel::NetworkKill).await);
        assert!(!ks.is_level_active(KillLevel::FullStop).await);

        // Escalate to NetworkKill
        ks.trigger(KillLevel::NetworkKill, "test", "test")
            .await;
        assert!(ks.is_level_active(KillLevel::Throttle).await);
        assert!(ks.is_level_active(KillLevel::ScopeBlock).await);
        assert!(ks.is_level_active(KillLevel::NetworkKill).await);
        assert!(!ks.is_level_active(KillLevel::FullStop).await);

        // Escalate to FullStop
        ks.trigger(KillLevel::FullStop, "test", "test")
            .await;
        assert!(ks.is_level_active(KillLevel::Throttle).await);
        assert!(ks.is_level_active(KillLevel::ScopeBlock).await);
        assert!(ks.is_level_active(KillLevel::NetworkKill).await);
        assert!(ks.is_level_active(KillLevel::FullStop).await);
    }

    #[tokio::test]
    async fn test_fullstop_blocks_all_tool_dispatch() {
        let ks = OrchestratorKillSwitch::new();
        ks.trigger(KillLevel::FullStop, "critical", "operator")
            .await;

        // FullStop should block even file tools
        let result = ks
            .check_tool_dispatch(
                "file_read",
                &serde_json::json!({"path": "/tmp/test.txt"}),
            )
            .await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), KillSwitchError::FullStop(_)));

        // And network tools
        let result = ks
            .check_tool_dispatch("http_request", &serde_json::json!({}))
            .await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), KillSwitchError::FullStop(_)));
    }

    #[tokio::test]
    async fn test_reset_clears_kill_switch() {
        let ks = OrchestratorKillSwitch::new();
        ks.trigger(KillLevel::FullStop, "test", "test").await;

        let result = ks.check_fullstop().await;
        assert!(result.is_err());

        ks.reset().await;

        let result = ks.check_fullstop().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_is_network_tool() {
        assert!(is_network_tool("http_request"));
        assert!(is_network_tool("fetch"));
        assert!(is_network_tool("web_search"));
        assert!(!is_network_tool("file_read"));
        assert!(!is_network_tool("shell"));
    }

    #[tokio::test]
    async fn test_is_file_tool() {
        assert!(is_file_tool("file_read"));
        assert!(is_file_tool("file_write"));
        assert!(is_file_tool("read_file"));
        assert!(!is_file_tool("shell"));
        assert!(!is_file_tool("http_request"));
    }

    #[tokio::test]
    async fn test_extract_path_argument() {
        // Test various path argument names
        let args = serde_json::json!({"path": "/tmp/test.txt"});
        assert_eq!(
            extract_path_argument("file_read", &args),
            Some("/tmp/test.txt".to_string())
        );

        let args = serde_json::json!({"file": "/tmp/test.txt"});
        assert_eq!(
            extract_path_argument("file_read", &args),
            Some("/tmp/test.txt".to_string())
        );

        let args = serde_json::json!({"target": "/tmp/test.txt"});
        assert_eq!(
            extract_path_argument("file_read", &args),
            Some("/tmp/test.txt".to_string())
        );

        let args = serde_json::json!({"no_path": "/tmp/test.txt"});
        assert_eq!(extract_path_argument("file_read", &args), None);
    }

    #[tokio::test]
    async fn test_kill_level_ordering() {
        // Verify the severity ordering: Throttle < ScopeBlock < NetworkKill < FullStop
        assert!(KillLevel::Throttle.severity() < KillLevel::ScopeBlock.severity());
        assert!(
            KillLevel::ScopeBlock.severity() < KillLevel::NetworkKill.severity()
        );
        assert!(KillLevel::NetworkKill.severity() < KillLevel::FullStop.severity());
    }

    #[tokio::test]
    async fn test_active_level_returns_correct_level() {
        let ks = OrchestratorKillSwitch::new();

        assert!(ks.active_level().await.is_none());

        ks.trigger(KillLevel::Throttle, "test", "test")
            .await;
        assert_eq!(ks.active_level().await, Some(KillLevel::Throttle));

        ks.trigger(KillLevel::FullStop, "test", "test")
            .await;
        assert_eq!(ks.active_level().await, Some(KillLevel::FullStop));
    }

    #[tokio::test]
    async fn test_tool_dispatch_with_scope_block_on_path() {
        let ks = OrchestratorKillSwitch::new();
        ks.set_scope_block(ScopeBlock {
            blocked_paths: vec!["/etc".to_string(), "/root".to_string()],
            blocked_patterns: vec!["*.pem".to_string(), "*.key".to_string()],
        })
        .await;
        ks.trigger(KillLevel::ScopeBlock, "test", "test")
            .await;

        // Should block /etc paths
        let result = ks
            .check_tool_dispatch(
                "file_read",
                &serde_json::json!({"path": "/etc/passwd"}),
            )
            .await;
        assert!(result.is_err());

        // Should block *.pem patterns
        let result = ks
            .check_tool_dispatch(
                "file_write",
                &serde_json::json!({"path": "/home/user/key.pem"}),
            )
            .await;
        assert!(result.is_err());

        // Should allow other paths
        let result = ks
            .check_tool_dispatch(
                "file_read",
                &serde_json::json!({"path": "/home/user/document.txt"}),
            )
            .await;
        assert!(result.is_ok());
    }

    // ========================================================================
    // Integration Tests with ExecutionController
    // ========================================================================
    // These tests require access to internal modules, so they're gated behind
    // a feature flag that allows test modules to access non-public items.
    // For now, we test the OrchestratorKillSwitch in isolation above.
}
