//! Kill switch system for emergency stop capabilities.
//!
//! This module provides multiple kill levels for safety control:
//! - **Full Stop**: Complete system halt
//! - **Network Kill**: Block all network operations
//! - **Scope Block**: Block operations on specific scopes/paths
//! - **Throttle**: Rate limiting operations
//!
//! External verification allows integration with external flag systems
//! (environment variables, files, or Redis for production).

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Kill switch levels with escalating severity
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KillLevel {
    /// Complete system halt - no operations allowed
    FullStop,
    /// Block all network operations
    NetworkKill,
    /// Block operations on specific scopes/paths
    ScopeBlock,
    /// Rate limiting - slow down operations
    Throttle,
}

impl KillLevel {
    /// Check if this level allows network operations
    pub fn allows_network(&self) -> bool {
        !matches!(self, KillLevel::FullStop | KillLevel::NetworkKill)
    }

    /// Check if this level allows file operations
    pub fn allows_file_ops(&self) -> bool {
        !matches!(self, KillLevel::FullStop)
    }

    /// Check if this level allows execution
    pub fn allows_execution(&self) -> bool {
        !matches!(self, KillLevel::FullStop)
    }

    /// Get the severity rank (higher = more severe)
    pub fn severity(&self) -> u8 {
        match self {
            KillLevel::Throttle => 1,
            KillLevel::ScopeBlock => 2,
            KillLevel::NetworkKill => 3,
            KillLevel::FullStop => 4,
        }
    }
}

impl std::fmt::Display for KillLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KillLevel::FullStop => write!(f, "full_stop"),
            KillLevel::NetworkKill => write!(f, "network_kill"),
            KillLevel::ScopeBlock => write!(f, "scope_block"),
            KillLevel::Throttle => write!(f, "throttle"),
        }
    }
}

/// Kill switch state including active level and trigger info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KillSwitchState {
    /// Whether the kill switch is active
    pub active: bool,
    /// Current kill level
    pub level: Option<KillLevel>,
    /// Reason for triggering
    pub reason: Option<String>,
    /// Who/what triggered it
    pub trigger: Option<String>,
    /// Timestamp when triggered
    pub triggered_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl KillSwitchState {
    pub fn new() -> Self {
        Self {
            active: false,
            level: None,
            reason: None,
            trigger: None,
            triggered_at: None,
        }
    }

    /// Trigger the kill switch at a specific level
    pub fn trigger(
        &mut self,
        level: KillLevel,
        reason: impl Into<String>,
        trigger: impl Into<String>,
    ) {
        let now = chrono::Utc::now();
        let reason_str = reason.into();
        let trigger_str = trigger.into();
        tracing::warn!(
            level = %level,
            reason = %reason_str,
            trigger = %trigger_str,
            "Kill switch triggered"
        );
        self.active = true;
        self.level = Some(level);
        self.reason = Some(reason_str);
        self.trigger = Some(trigger_str);
        self.triggered_at = Some(now);
    }

    /// Reset the kill switch
    pub fn reset(&mut self) {
        tracing::info!("Kill switch reset");
        self.active = false;
        self.level = None;
        self.reason = None;
        self.trigger = None;
        self.triggered_at = None;
    }
}

impl Default for KillSwitchState {
    fn default() -> Self {
        Self::new()
    }
}

/// Scope block configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScopeBlock {
    /// Paths that are blocked
    pub blocked_paths: Vec<String>,
    /// File patterns to block (e.g., "*.pem", "*.env")
    pub blocked_patterns: Vec<String>,
}

/// Throttle configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThrottleConfig {
    /// Max operations per minute
    pub max_ops_per_minute: u32,
    /// Max concurrent operations
    pub max_concurrent: u32,
    /// Delay between operations in ms
    pub delay_ms: u64,
}

impl Default for ThrottleConfig {
    fn default() -> Self {
        Self {
            max_ops_per_minute: 10,
            max_concurrent: 2,
            delay_ms: 5000,
        }
    }
}

/// External verifier trait for kill switch state
pub trait KillSwitchVerifier: Send + Sync {
    /// Verify the current kill switch state from external source
    fn verify(&self) -> Option<KillLevel>;
}

/// Environment variable based verifier
#[derive(Debug, Clone)]
pub struct EnvVarVerifier {
    /// Environment variable name for kill switch level
    var_name: String,
}

impl EnvVarVerifier {
    pub fn new(var_name: impl Into<String>) -> Self {
        Self {
            var_name: var_name.into(),
        }
    }

    fn parse_level(value: &str) -> Option<KillLevel> {
        match value.trim().to_lowercase().as_str() {
            "full_stop" | "1" | "true" => Some(KillLevel::FullStop),
            "network_kill" | "2" => Some(KillLevel::NetworkKill),
            "scope_block" | "3" => Some(KillLevel::ScopeBlock),
            "throttle" | "4" => Some(KillLevel::Throttle),
            _ => None,
        }
    }
}

impl KillSwitchVerifier for EnvVarVerifier {
    fn verify(&self) -> Option<KillLevel> {
        std::env::var(&self.var_name)
            .ok()
            .and_then(|v| Self::parse_level(&v))
    }
}

/// File-based verifier for kill switch state
#[derive(Debug, Clone)]
pub struct FileVerifier {
    /// Path to flag file
    file_path: std::path::PathBuf,
}

impl FileVerifier {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            file_path: path.into(),
        }
    }

    fn parse_level(content: &str) -> Option<KillLevel> {
        let trimmed = content.trim();
        if trimmed.is_empty() {
            return None;
        }
        EnvVarVerifier::parse_level(trimmed)
    }
}

impl KillSwitchVerifier for FileVerifier {
    fn verify(&self) -> Option<KillLevel> {
        std::fs::read_to_string(&self.file_path)
            .ok()
            .and_then(|c| Self::parse_level(&c))
    }
}

/// The main kill switch guard that orchestrator checks before operations
pub struct KillSwitchGuard {
    state: Arc<RwLock<KillSwitchState>>,
    scope_block: Arc<RwLock<ScopeBlock>>,
    throttle: Arc<RwLock<ThrottleConfig>>,
    verifiers: Vec<Box<dyn KillSwitchVerifier>>,
    last_verified: Arc<RwLock<Option<chrono::DateTime<chrono::Utc>>>>,
}

impl KillSwitchGuard {
    /// Create a new kill switch guard with default settings
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(KillSwitchState::new())),
            scope_block: Arc::new(RwLock::new(ScopeBlock::default())),
            throttle: Arc::new(RwLock::new(ThrottleConfig::default())),
            verifiers: Vec::new(),
            last_verified: Arc::new(RwLock::new(None)),
        }
    }

    /// Add an external verifier
    pub fn with_verifier(mut self, verifier: Box<dyn KillSwitchVerifier>) -> Self {
        self.verifiers.push(verifier);
        self
    }

    /// Add environment variable verifier
    pub fn with_env_verifier(mut self, var_name: impl Into<String>) -> Self {
        self.verifiers.push(Box::new(EnvVarVerifier::new(var_name)));
        self
    }

    /// Add file verifier
    pub fn with_file_verifier(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.verifiers.push(Box::new(FileVerifier::new(path)));
        self
    }

    /// Set scope block configuration
    pub async fn set_scope_block(&self, config: ScopeBlock) {
        let mut sb = self.scope_block.write().await;
        *sb = config;
    }

    /// Set throttle configuration
    pub async fn set_throttle(&self, config: ThrottleConfig) {
        let mut t = self.throttle.write().await;
        *t = config;
    }

    /// Manually trigger the kill switch
    pub async fn trigger(
        &self,
        level: KillLevel,
        reason: impl Into<String>,
        trigger: impl Into<String>,
    ) {
        let mut state = self.state.write().await;
        state.trigger(level, reason, trigger);
    }

    /// Reset the kill switch
    pub async fn reset(&self) {
        let mut state = self.state.write().await;
        state.reset();
    }

    /// Verify external sources and update state if needed
    pub async fn verify_external(&self) {
        for verifier in &self.verifiers {
            if let Some(level) = verifier.verify() {
                let mut state = self.state.write().await;
                if !state.active || state.level.map(|l| l.severity()) < Some(level.severity()) {
                    state.trigger(
                        level,
                        format!("External: {:?}", std::any::type_name::<Self>()),
                        "external",
                    );
                }
            }
        }
        let mut last = self.last_verified.write().await;
        *last = Some(chrono::Utc::now());
    }

    /// Get current kill switch state
    pub async fn state(&self) -> KillSwitchState {
        self.state.read().await.clone()
    }

    /// Check if operations are allowed
    pub async fn check(&self) -> Result<(), KillSwitchError> {
        // First verify external sources
        self.verify_external().await;

        let state = self.state.read().await;
        if !state.active {
            return Ok(());
        }

        let level = state.level.ok_or(KillSwitchError::Unknown)?;

        match level {
            KillLevel::FullStop => Err(KillSwitchError::FullStop(state.reason.clone())),
            KillLevel::NetworkKill => Err(KillSwitchError::NetworkKilled(state.reason.clone())),
            KillLevel::ScopeBlock => Err(KillSwitchError::ScopeBlocked(state.reason.clone())),
            KillLevel::Throttle => Err(KillSwitchError::Throttled(state.reason.clone())),
        }
    }

    /// Check if network operations are allowed
    pub async fn check_network(&self) -> Result<(), KillSwitchError> {
        let state = self.state.read().await;
        if !state.active {
            return Ok(());
        }

        let level = state.level.ok_or(KillSwitchError::Unknown)?;

        if !level.allows_network() {
            return Err(KillSwitchError::NetworkKilled(state.reason.clone()));
        }
        Ok(())
    }

    /// Check if file operations on a path are allowed
    pub async fn check_path(&self, path: &str) -> Result<(), KillSwitchError> {
        let state = self.state.read().await;
        if !state.active {
            return Ok(());
        }

        let level = state.level.ok_or(KillSwitchError::Unknown)?;

        if !level.allows_file_ops() {
            return Err(KillSwitchError::ScopeBlocked(state.reason.clone()));
        }

        // Check scope blocks
        if level == KillLevel::ScopeBlock {
            let sb = self.scope_block.read().await;
            if sb.blocked_paths.iter().any(|p| path.starts_with(p)) {
                return Err(KillSwitchError::ScopeBlocked(state.reason.clone()));
            }
            for pattern in &sb.blocked_patterns {
                if glob_match(pattern, path) {
                    return Err(KillSwitchError::ScopeBlocked(state.reason.clone()));
                }
            }
        }
        Ok(())
    }

    /// Check throttle status and get delay if throttled
    pub async fn check_throttle(&self) -> Result<(), KillSwitchError> {
        let state = self.state.read().await;
        if !state.active {
            return Ok(());
        }

        let level = state.level.ok_or(KillSwitchError::Unknown)?;

        if level == KillLevel::Throttle {
            let config = self.throttle.read().await;
            Err(KillSwitchError::ThrottledWithDelay {
                reason: state.reason.clone(),
                delay_ms: config.delay_ms,
            })
        } else {
            Ok(())
        }
    }

    /// Get last verification time
    pub async fn last_verified(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        *self.last_verified.read().await
    }
}

impl Default for KillSwitchGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple glob pattern matching
fn glob_match(pattern: &str, path: &str) -> bool {
    // Simple glob matching for file patterns like *.pem, *.env
    let pattern = pattern.trim();
    let path = path.trim();

    if let Some(ext) = pattern.strip_prefix("*.") {
        path.ends_with(&format!(".{}", ext))
    } else if let Some(prefix) = pattern.strip_suffix(".*") {
        path.starts_with(prefix)
    } else {
        path == pattern
    }
}

/// Kill switch errors
#[derive(Debug, thiserror::Error)]
pub enum KillSwitchError {
    #[error("System full stop: {}", .0.as_deref().unwrap_or("unknown"))]
    FullStop(Option<String>),

    #[error("Network operations killed: {}", .0.as_deref().unwrap_or("unknown"))]
    NetworkKilled(Option<String>),

    #[error("Scope blocked: {}", .0.as_deref().unwrap_or("unknown"))]
    ScopeBlocked(Option<String>),

    #[error("Throttled: {}", .0.as_deref().unwrap_or("unknown"))]
    Throttled(Option<String>),

    #[error("Throttled with delay: {} ({}ms delay)", .reason.as_deref().unwrap_or("unknown"), .delay_ms)]
    ThrottledWithDelay {
        reason: Option<String>,
        delay_ms: u64,
    },

    #[error("Unknown kill switch state")]
    Unknown,
}

impl KillSwitchError {
    pub fn reason(&self) -> Option<&str> {
        match self {
            KillSwitchError::FullStop(r) => r.as_deref(),
            KillSwitchError::NetworkKilled(r) => r.as_deref(),
            KillSwitchError::ScopeBlocked(r) => r.as_deref(),
            KillSwitchError::Throttled(r) => r.as_deref(),
            KillSwitchError::ThrottledWithDelay { reason: r, .. } => r.as_deref(),
            KillSwitchError::Unknown => None,
        }
    }

    pub fn is_blocking(&self) -> bool {
        matches!(
            self,
            KillSwitchError::FullStop(_)
                | KillSwitchError::NetworkKilled(_)
                | KillSwitchError::ScopeBlocked(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kill_level_allows_network() {
        assert!(!KillLevel::FullStop.allows_network());
        assert!(!KillLevel::NetworkKill.allows_network());
        assert!(KillLevel::ScopeBlock.allows_network());
        assert!(KillLevel::Throttle.allows_network());
    }

    #[test]
    fn test_kill_level_allows_file_ops() {
        assert!(!KillLevel::FullStop.allows_file_ops());
        assert!(KillLevel::NetworkKill.allows_file_ops());
        assert!(KillLevel::ScopeBlock.allows_file_ops());
        assert!(KillLevel::Throttle.allows_file_ops());
    }

    #[test]
    fn test_kill_level_severity() {
        assert!(KillLevel::FullStop.severity() > KillLevel::NetworkKill.severity());
        assert!(KillLevel::NetworkKill.severity() > KillLevel::ScopeBlock.severity());
        assert!(KillLevel::ScopeBlock.severity() > KillLevel::Throttle.severity());
    }

    #[test]
    fn test_kill_switch_state_trigger() {
        let mut state = KillSwitchState::new();
        assert!(!state.active);

        state.trigger(KillLevel::FullStop, "Test reason", "test");

        assert!(state.active);
        assert_eq!(state.level, Some(KillLevel::FullStop));
        assert_eq!(state.reason, Some("Test reason".to_string()));
        assert_eq!(state.trigger, Some("test".to_string()));
        assert!(state.triggered_at.is_some());
    }

    #[test]
    fn test_kill_switch_state_reset() {
        let mut state = KillSwitchState::new();
        state.trigger(KillLevel::FullStop, "Test", "test");
        assert!(state.active);

        state.reset();

        assert!(!state.active);
        assert!(state.level.is_none());
        assert!(state.reason.is_none());
    }

    #[tokio::test]
    async fn test_kill_switch_guard_check_active() {
        let guard = KillSwitchGuard::new();
        guard.trigger(KillLevel::FullStop, "test", "test").await;

        let result = guard.check().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_kill_switch_guard_check_inactive() {
        let guard = KillSwitchGuard::new();
        let result = guard.check().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_kill_switch_guard_network_kill() {
        let guard = KillSwitchGuard::new();
        guard.trigger(KillLevel::NetworkKill, "test", "test").await;

        let result = guard.check_network().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_kill_switch_guard_scope_block() {
        let guard = KillSwitchGuard::new();
        guard.trigger(KillLevel::ScopeBlock, "test", "test").await;
        guard
            .set_scope_block(ScopeBlock {
                blocked_paths: vec!["/secret".to_string()],
                blocked_patterns: vec![],
            })
            .await;

        let result = guard.check_path("/secret/file.txt").await;
        assert!(result.is_err());

        let result = guard.check_path("/allowed/file.txt").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_kill_switch_guard_throttle() {
        let guard = KillSwitchGuard::new();
        guard.trigger(KillLevel::Throttle, "test", "test").await;

        let result = guard.check_throttle().await;
        assert!(matches!(
            result,
            Err(KillSwitchError::ThrottledWithDelay { .. })
        ));
    }

    #[test]
    fn test_glob_match() {
        assert!(glob_match("*.pem", "key.pem"));
        assert!(glob_match("*.pem", "certificate.pem"));
        assert!(!glob_match("*.pem", "key.txt"));
        assert!(glob_match("*.env", ".env"));
        assert!(glob_match("*.env", "config.env"));
        assert!(glob_match("test.*", "test.txt"));
        assert!(glob_match("test.*", "test.rs"));
        assert!(!glob_match("test.*", "other.txt"));
    }

    #[tokio::test]
    async fn test_kill_switch_error_reason() {
        let guard = KillSwitchGuard::new();
        guard
            .trigger(KillLevel::FullStop, "Critical failure", "operator")
            .await;

        let result = guard.check().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.reason(), Some("Critical failure"));
    }

    #[tokio::test]
    async fn test_kill_switch_multiple_levels_escalate() {
        let guard = KillSwitchGuard::new();

        // First trigger at throttle level
        guard
            .trigger(KillLevel::Throttle, "minor issue", "auto")
            .await;
        let state = guard.state().await;
        assert_eq!(state.level, Some(KillLevel::Throttle));

        // Manually escalate to full stop
        guard
            .trigger(KillLevel::FullStop, "critical issue", "operator")
            .await;
        let state = guard.state().await;
        assert_eq!(state.level, Some(KillLevel::FullStop));
    }

    #[tokio::test]
    async fn test_scope_block_patterns() {
        let guard = KillSwitchGuard::new();
        guard
            .set_scope_block(ScopeBlock {
                blocked_paths: vec![],
                blocked_patterns: vec!["*.pem".to_string(), "*.env".to_string()],
            })
            .await;

        assert!(glob_match("*.pem", "key.pem"));
        assert!(glob_match("*.env", ".env"));

        // Without active kill switch, path check passes
        let result = guard.check_path("key.pem").await;
        assert!(result.is_ok());

        // Activate scope block
        guard
            .trigger(KillLevel::ScopeBlock, "sensitive files", "policy")
            .await;

        let result = guard.check_path("key.pem").await;
        assert!(result.is_err());

        let result = guard.check_path("safe.rs").await;
        assert!(result.is_ok());
    }
}
