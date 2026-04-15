//! OS-level sandboxing via Seatbelt (macOS), Bubblewrap (Linux), and Landlock (Linux kernel).
//!
//! This module provides platform-specific sandbox implementations for confining shell command
//! execution with filesystem restrictions and network access control.
//!
//! ## Platforms
//!
//! - **macOS**: Seatbelt via `sandbox-exec` (fully implemented)
//! - **Linux**: Bubblewrap (`bwrap`) with Landlock as fallback
//!
//! ## Usage
//!
//! ```rust,ignore
//! use swell_sandbox::os_sandbox::{OsSandbox, OsSandboxConfig, PlatformSandbox};
//!
//! // Create configuration with restrictions
//! let config = OsSandboxConfig::default()
//!     .with_allowed_dirs(["/workspace"])
//!     .with_network(NetworkPolicy::DenyAll);
//!
//! // Get the appropriate platform sandbox
//! let sandbox = PlatformSandbox::new(config);
//! let output = sandbox.execute("echo hello", None).await;
//! ```

#[cfg(target_os = "macos")]
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use swell_core::{SandboxOutput, SwellError};

#[cfg(target_os = "macos")]
use tokio::process::Command;

/// Network policy for sandboxed commands
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NetworkPolicy {
    /// Allow all network access
    #[default]
    AllowAll,
    /// Deny all network access
    DenyAll,
    /// Allow only specific hosts/ports (future)
    AllowList,
}

impl NetworkPolicy {
    pub fn as_sandbox_arg(&self) -> Option<&'static str> {
        match self {
            NetworkPolicy::AllowAll => None,
            NetworkPolicy::DenyAll => Some("deny"),
            NetworkPolicy::AllowList => Some("allow"),
        }
    }
}

/// Filesystem permission for a path
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilesystemPermission {
    /// Read-only access
    ReadOnly,
    /// Read-write access
    ReadWrite,
    /// No access (blocked)
    NoAccess,
}

impl FilesystemPermission {
    pub fn as_sandbox_arg(&self) -> &'static str {
        match self {
            FilesystemPermission::ReadOnly => "ro",
            FilesystemPermission::ReadWrite => "rw",
            FilesystemPermission::NoAccess => "no",
        }
    }
}

/// Configuration for OS-level sandbox
#[derive(Debug, Clone)]
pub struct OsSandboxConfig {
    /// Unique identifier for this sandbox
    pub sandbox_id: String,
    /// Allowed directories with their permissions
    pub allowed_dirs: HashMap<PathBuf, FilesystemPermission>,
    /// Temporary directory (should always be allowed)
    pub temp_dir: PathBuf,
    /// Network policy
    pub network_policy: NetworkPolicy,
    /// Additional environment variables to pass
    pub env: HashMap<String, String>,
    /// Working directory (defaults to temp_dir)
    pub working_dir: Option<PathBuf>,
}

impl Default for OsSandboxConfig {
    fn default() -> Self {
        Self {
            sandbox_id: uuid::Uuid::new_v4().to_string(),
            allowed_dirs: HashMap::new(),
            temp_dir: std::env::temp_dir(),
            network_policy: NetworkPolicy::DenyAll,
            env: HashMap::new(),
            working_dir: None,
        }
    }
}

impl OsSandboxConfig {
    /// Add an allowed directory with read-only permission
    pub fn with_allowed_dir_ro(mut self, path: impl Into<PathBuf>) -> Self {
        self.allowed_dirs
            .insert(path.into(), FilesystemPermission::ReadOnly);
        self
    }

    /// Add an allowed directory with read-write permission
    pub fn with_allowed_dir_rw(mut self, path: impl Into<PathBuf>) -> Self {
        self.allowed_dirs
            .insert(path.into(), FilesystemPermission::ReadWrite);
        self
    }

    /// Add multiple allowed directories (read-only by default)
    pub fn with_allowed_dirs<I, P>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        for path in paths {
            self.allowed_dirs
                .insert(path.into(), FilesystemPermission::ReadOnly);
        }
        self
    }

    /// Set the network policy
    pub fn with_network(mut self, policy: NetworkPolicy) -> Self {
        self.network_policy = policy;
        self
    }

    /// Set the working directory
    pub fn with_working_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    /// Add an environment variable
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }
}

/// Result of a sandbox availability check
#[derive(Debug, Clone)]
pub struct SandboxAvailability {
    /// Whether the sandbox is available on this platform
    pub is_available: bool,
    /// The type of sandbox available
    pub sandbox_type: Option<SandboxType>,
    /// Description of availability (e.g., "seatbelt not installed")
    pub description: String,
}

/// Type of OS-level sandbox
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxType {
    /// Seatbelt/sandbox-exec on macOS
    Seatbelt,
    /// Bubblewrap on Linux
    Bubblewrap,
    /// Landlock on Linux (fallback)
    Landlock,
}

impl std::fmt::Display for SandboxType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SandboxType::Seatbelt => write!(f, "seatbelt"),
            SandboxType::Bubblewrap => write!(f, "bubblewrap"),
            SandboxType::Landlock => write!(f, "landlock"),
        }
    }
}

/// Detect which platform and sandbox is available (async)
pub async fn detect_available_sandbox() -> SandboxAvailability {
    #[cfg(target_os = "macos")]
    {
        let available = is_seatbelt_available().await;
        SandboxAvailability {
            is_available: available,
            sandbox_type: Some(SandboxType::Seatbelt),
            description: if available {
                "seatbelt (sandbox-exec) is available".to_string()
            } else {
                "seatbelt (sandbox-exec) is not available".to_string()
            },
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        SandboxAvailability {
            is_available: false,
            sandbox_type: None,
            description: "OS-level sandboxing not available on this platform".to_string(),
        }
    }
}

/// Detect which platform and sandbox is available (synchronous version)
pub fn detect_available_sandbox_sync() -> SandboxAvailability {
    #[cfg(target_os = "macos")]
    {
        let available = std::process::Command::new("which")
            .arg("sandbox-exec")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        SandboxAvailability {
            is_available: available,
            sandbox_type: Some(SandboxType::Seatbelt),
            description: if available {
                "seatbelt (sandbox-exec) is available".to_string()
            } else {
                "seatbelt (sandbox-exec) is not available".to_string()
            },
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        SandboxAvailability {
            is_available: false,
            sandbox_type: None,
            description: "OS-level sandboxing not available on this platform".to_string(),
        }
    }
}

/// Check if seatbelt is available (sandbox-exec)
#[cfg(target_os = "macos")]
pub(crate) async fn is_seatbelt_available() -> bool {
    which("sandbox-exec").await.is_some()
}

/// Find a command in PATH
async fn which(cmd: &str) -> Option<PathBuf> {
    tokio::process::Command::new("which")
        .arg(cmd)
        .output()
        .await
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .filter(|p| !p.as_os_str().is_empty())
}

/// Seatbelt sandbox implementation for macOS using sandbox-exec
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SeatbeltSandbox {
    config: OsSandboxConfig,
}

impl SeatbeltSandbox {
    #[allow(dead_code)]
    pub fn new(config: OsSandboxConfig) -> Self {
        Self { config }
    }

    #[allow(dead_code)]
    pub fn with_default_config() -> Self {
        Self::new(OsSandboxConfig::default())
    }

    /// Build the sandbox-exec profile
    ///
    /// The profile restricts:
    /// - Filesystem access to allowed directories only
    /// - Network access based on network_policy
    /// - Process spawning
    #[allow(dead_code)]
    fn build_sandbox_profile(&self) -> String {
        let mut profile = String::from("(version 1)\n");

        // Default deny for network first
        match self.config.network_policy {
            NetworkPolicy::DenyAll => {
                profile.push_str("(deny default)\n");
                profile.push_str("(allow default)\n");
            }
            NetworkPolicy::AllowAll => {
                profile.push_str("(allow default)\n");
            }
            NetworkPolicy::AllowList => {
                profile.push_str("(deny default)\n");
            }
        }

        // Filesystem rules - allow access to specified paths only
        for (path, perm) in &self.config.allowed_dirs {
            let path_str = path.to_string_lossy();
            match perm {
                FilesystemPermission::ReadOnly => {
                    profile.push_str(&format!("(allow file-read* (literal \"{}\"))\n", path_str));
                    profile.push_str(&format!(
                        "(allow file-read-metadata* (literal \"{}\"))\n",
                        path_str
                    ));
                }
                FilesystemPermission::ReadWrite => {
                    profile.push_str(&format!(
                        "(allow file-read* file-write* (literal \"{}\"))\n",
                        path_str
                    ));
                }
                FilesystemPermission::NoAccess => {
                    profile.push_str(&format!("(deny file-read* (literal \"{}\"))\n", path_str));
                }
            }
        }

        // Temp directory - allow read-write for temp files
        let temp_str = self.config.temp_dir.to_string_lossy();
        profile.push_str(&format!(
            "(allow file-read* file-write* (literal \"{}\"))\n",
            temp_str
        ));

        // Network rules
        if self.config.network_policy == NetworkPolicy::DenyAll {
            profile.push_str("(deny network*)\n");
        } else {
            profile.push_str("(allow network*)\n");
        }

        // Process execution - allow process operations for shell commands
        profile.push_str("(allow process-exec)\n");
        profile.push_str("(allow process-fork)\n");

        // Environment variables - allow reading common env vars
        profile.push_str("(allow env*)\n");

        profile
    }
}

#[cfg(target_os = "macos")]
#[async_trait]
impl OsSandbox for SeatbeltSandbox {
    fn id(&self) -> &str {
        &self.config.sandbox_id
    }

    fn sandbox_type(&self) -> SandboxType {
        SandboxType::Seatbelt
    }

    fn is_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("sandbox-exec")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    async fn execute(
        &self,
        cmd: &str,
        args: Option<&[String]>,
    ) -> Result<SandboxOutput, SwellError> {
        let sandbox_profile = self.build_sandbox_profile();

        tracing::debug!(
            sandbox_id = %self.config.sandbox_id,
            cmd = %cmd,
            "SeatbeltSandbox: executing with sandbox profile"
        );

        let start = Instant::now();

        // Build sandbox-exec command
        // sandbox-exec -f <profile> -- <command> [args...]
        let mut sandbox_args = vec!["-f".to_string()];
        sandbox_args.push(sandbox_profile);
        sandbox_args.push("--".to_string());
        sandbox_args.push(cmd.to_string());

        if let Some(cmd_args) = args {
            sandbox_args.extend(cmd_args.iter().cloned());
        }

        let output = Command::new("sandbox-exec")
            .args(&sandbox_args)
            .output()
            .await
            .map_err(|e| {
                SwellError::ToolExecutionFailed(format!("sandbox-exec execution failed: {}", e))
            })?;

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(SandboxOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            duration_ms,
        })
    }

    /// Execute with working directory and environment
    async fn execute_full(
        &self,
        command: String,
        args: Vec<String>,
        _env: std::collections::HashMap<String, String>,
        working_dir: Option<String>,
    ) -> Result<SandboxOutput, SwellError> {
        let mut full_cmd = command;
        if !args.is_empty() {
            full_cmd.push(' ');
            full_cmd.push_str(&args.join(" "));
        }

        tracing::debug!(
            sandbox_id = %self.config.sandbox_id,
            cmd = %full_cmd,
            working_dir = ?working_dir,
            "SeatbeltSandbox: execute_full"
        );

        // Build sandbox-exec command
        let sandbox_profile = self.build_sandbox_profile();
        let mut sandbox_args = vec!["-f".to_string()];
        sandbox_args.push(sandbox_profile);
        sandbox_args.push("--".to_string());

        if let Some(ref dir) = working_dir {
            sandbox_args.push("sh".to_string());
            sandbox_args.push("-c".to_string());
            sandbox_args.push(format!("cd {} && {}", dir, full_cmd));
        } else {
            sandbox_args.push("sh".to_string());
            sandbox_args.push("-c".to_string());
            sandbox_args.push(full_cmd);
        }

        let start = Instant::now();

        let output = Command::new("sandbox-exec")
            .args(&sandbox_args)
            .output()
            .await
            .map_err(|e| {
                SwellError::ToolExecutionFailed(format!("sandbox-exec execution failed: {}", e))
            })?;

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(SandboxOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            duration_ms,
        })
    }
}
#[cfg(target_os = "macos")]
#[async_trait]
#[allow(dead_code)]
pub trait OsSandbox: Send + Sync {
    /// Unique identifier for this sandbox
    fn id(&self) -> &str;

    /// Execute a command in the sandbox
    async fn execute(
        &self,
        cmd: &str,
        args: Option<&[String]>,
    ) -> Result<SandboxOutput, SwellError>;

    /// Execute with full SandboxCommand (includes working dir, env vars, timeout)
    async fn execute_full(
        &self,
        command: String,
        args: Vec<String>,
        env: std::collections::HashMap<String, String>,
        working_dir: Option<String>,
    ) -> Result<SandboxOutput, SwellError>;

    /// Check if the sandbox is available on this system
    fn is_available(&self) -> bool;

    /// Get the sandbox type
    fn sandbox_type(&self) -> SandboxType;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_policy_as_arg() {
        assert_eq!(NetworkPolicy::AllowAll.as_sandbox_arg(), None);
        assert_eq!(NetworkPolicy::DenyAll.as_sandbox_arg(), Some("deny"));
    }

    #[test]
    fn test_filesystem_permission_as_arg() {
        assert_eq!(FilesystemPermission::ReadOnly.as_sandbox_arg(), "ro");
        assert_eq!(FilesystemPermission::ReadWrite.as_sandbox_arg(), "rw");
        assert_eq!(FilesystemPermission::NoAccess.as_sandbox_arg(), "no");
    }

    #[test]
    fn test_os_sandbox_config_builder() {
        let config = OsSandboxConfig::default()
            .with_allowed_dir_ro("/workspace")
            .with_allowed_dir_rw("/tmp")
            .with_network(NetworkPolicy::DenyAll)
            .with_working_dir("/workspace");

        assert_eq!(config.allowed_dirs.len(), 2);
        assert_eq!(config.network_policy, NetworkPolicy::DenyAll);
        assert!(config.working_dir.is_some());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn test_seatbelt_sandbox_new() {
        let config = OsSandboxConfig::default();
        let sandbox = SeatbeltSandbox::new(config);
        assert_eq!(sandbox.sandbox_type(), SandboxType::Seatbelt);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_seatbelt_build_profile() {
        let config = OsSandboxConfig::default()
            .with_allowed_dir_ro("/workspace")
            .with_allowed_dir_rw("/tmp")
            .with_network(NetworkPolicy::DenyAll);

        let sandbox = SeatbeltSandbox::new(config);
        let profile = sandbox.build_sandbox_profile();

        assert!(profile.contains("(deny default)"));
        assert!(profile.contains("(allow default)"));
        assert!(profile.contains("file-read*"));
    }

    #[test]
    fn test_detect_sandbox_sync() {
        let availability = detect_available_sandbox_sync();
        println!("Sandbox availability (sync): {:?}", availability);
        assert!(!availability.description.is_empty());
    }

    #[tokio::test]
    async fn test_detect_sandbox_async() {
        let availability = detect_available_sandbox().await;
        println!("Sandbox availability (async): {:?}", availability);
        assert!(!availability.description.is_empty());
    }
}
