//! External kill switch integration for SWELL tools.
//!
//! This module provides Redis-based kill switch functionality for tools,
//! allowing emergency stop of tool operations from an external control plane.
//!
//! # Architecture
//!
//! The kill switch system consists of:
//! - **RedisVerifier**: Polls Redis for kill signals
//! - **KillSwitchGuard**: Manages kill state and policy checks
//!
//! # Usage
//!
//! ```rust,ignore
//! use swell_core::kill_switch::{KillSwitchGuard, RedisVerifier, KillLevel};
//! use swell_tools::killswitch::ToolKillSwitch;
//!
//! // Create a kill switch guard with Redis verification
//! let guard = KillSwitchGuard::new()
//!     .with_redis_verifier("redis://localhost:6379", "swell:kill_switch");
//!
//! let tool_kill_switch = ToolKillSwitch::new(guard);
//!
//! // Check before executing a tool
//! tool_kill_switch.check().await?;
//! ```

use swell_core::kill_switch::{KillLevel, KillSwitchGuard};

/// Tool-level kill switch wrapper that integrates with the core kill switch system.
///
/// This provides a simplified interface for tools to check the kill switch
/// before executing operations.
pub struct ToolKillSwitch {
    /// The underlying kill switch guard
    guard: KillSwitchGuard,
}

impl ToolKillSwitch {
    /// Create a new tool kill switch with the given guard
    pub fn new(guard: KillSwitchGuard) -> Self {
        Self { guard }
    }

    /// Create a tool kill switch with Redis-based external verification
    pub fn with_redis(redis_url: impl Into<String>, key: impl Into<String>) -> Self {
        let guard = KillSwitchGuard::new().with_redis_verifier(redis_url, key);
        Self { guard }
    }

    /// Check if operations are allowed (full stop check)
    pub async fn check(&self) -> Result<(), swell_core::kill_switch::KillSwitchError> {
        self.guard.check().await
    }

    /// Check if operations are allowed, verifying Redis
    pub async fn check_with_verification(
        &self,
    ) -> Result<(), swell_core::kill_switch::KillSwitchError> {
        self.guard.verify_external().await;
        self.guard.check().await
    }

    /// Get current kill switch state
    pub async fn state(&self) -> swell_core::kill_switch::KillSwitchState {
        self.guard.state().await
    }

    /// Manually trigger the kill switch
    pub async fn trigger(
        &self,
        level: KillLevel,
        reason: impl Into<String>,
        trigger: impl Into<String>,
    ) {
        self.guard.trigger(level, reason, trigger).await;
    }

    /// Reset the kill switch
    pub async fn reset(&self) {
        self.guard.reset().await;
    }
}

impl std::fmt::Debug for ToolKillSwitch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolKillSwitch").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swell_core::kill_switch::KillLevel;

    #[tokio::test]
    async fn test_tool_kill_switch_check_inactive() {
        let guard = KillSwitchGuard::new();
        let tool_kill_switch = ToolKillSwitch::new(guard);

        let result = tool_kill_switch.check().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_tool_kill_switch_trigger_full_stop() {
        let guard = KillSwitchGuard::new();
        let tool_kill_switch = ToolKillSwitch::new(guard);

        tool_kill_switch
            .trigger(KillLevel::FullStop, "test", "test")
            .await;

        let result = tool_kill_switch.check().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_tool_kill_switch_with_redis() {
        // This test verifies that the Redis verifier can be added
        // Note: Without Redis running, we test the guard creation and manual trigger
        let tool_kill_switch =
            ToolKillSwitch::with_redis("redis://localhost:6379", "swell:kill_switch");

        // Without any trigger, check should pass even if Redis isn't running
        // (since no kill signal is active)
        let result = tool_kill_switch.check().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_tool_kill_switch_state() {
        let guard = KillSwitchGuard::new();
        let tool_kill_switch = ToolKillSwitch::new(guard);

        let state = tool_kill_switch.state().await;
        assert!(!state.active);
    }
}
