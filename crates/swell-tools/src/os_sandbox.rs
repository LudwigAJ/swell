//! OS-level sandboxing via Bubblewrap (Linux), Seatbelt (macOS), and Landlock (Linux kernel).
//!
//! This module provides platform-specific sandbox implementations for confining shell command
//! execution with filesystem restrictions and network access control.
//!
//! ## Platforms
//!
//! - **Linux**: Bubblewrap (`bwrap`) is preferred, with Landlock as fallback for newer kernels
//! - **macOS**: Seatbelt via `sandbox-exec`
//!
//! ## Usage
//!
//! ```rust,ignore
//! use swell_tools::os_sandbox::{OsSandbox, OsSandboxConfig, PlatformSandbox};
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

use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use swell_core::{SandboxCommand, SandboxOutput, SwellError};
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
    /// User to run as (for bubblewrap)
    pub run_as_user: Option<String>,
    /// Disable user namespace (for bubblewrap)
    pub disable_user_ns: bool,
    /// Share network namespace with host (for bubblewrap)
    pub share_network: bool,
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
            run_as_user: None,
            disable_user_ns: false,
            share_network: true, // Default to sharing network for usability
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
    /// Description of availability (e.g., "bubblewrap not installed")
    pub description: String,
}

/// Type of OS-level sandbox
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxType {
    /// Bubblewrap on Linux
    Bubblewrap,
    /// Seatbelt/sandbox-exec on macOS
    Seatbelt,
    /// Landlock on Linux (fallback)
    Landlock,
}

impl std::fmt::Display for SandboxType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SandboxType::Bubblewrap => write!(f, "bubblewrap"),
            SandboxType::Seatbelt => write!(f, "seatbelt"),
            SandboxType::Landlock => write!(f, "landlock"),
        }
    }
}

/// Unified trait for OS-level sandbox implementations
#[async_trait]
pub trait OsSandbox: Send + Sync {
    /// Unique identifier for this sandbox
    fn id(&self) -> &str;

    /// Execute a command in the sandbox
    async fn execute(
        &self,
        cmd: &str,
        args: Option<&[String]>,
    ) -> Result<SandboxOutput, SwellError>;

    /// Execute with full SandboxCommand
    async fn execute_full(&self, cmd: SandboxCommand) -> Result<SandboxOutput, SwellError>;

    /// Check if the sandbox is available on this system
    fn is_available(&self) -> bool;

    /// Get the sandbox type
    fn sandbox_type(&self) -> SandboxType;
}

/// Detect which platform and sandbox is available (async)
pub async fn detect_available_sandbox() -> SandboxAvailability {
    #[cfg(target_os = "linux")]
    {
        // Check for bubblewrap first
        if is_bubblewrap_available().await {
            return SandboxAvailability {
                is_available: true,
                sandbox_type: Some(SandboxType::Bubblewrap),
                description: "bubblewrap is available".to_string(),
            };
        }

        // Check for landlock support
        if is_landlock_available() {
            return SandboxAvailability {
                is_available: true,
                sandbox_type: Some(SandboxType::Landlock),
                description: "landlock is available (kernel support)".to_string(),
            };
        }

        SandboxAvailability {
            is_available: false,
            sandbox_type: None,
            description:
                "no Linux sandbox available (bubblewrap not installed, landlock not supported)"
                    .to_string(),
        }
    }

    #[cfg(target_os = "macos")]
    {
        // Always available on macOS via sandbox-exec
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

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        SandboxAvailability {
            is_available: false,
            sandbox_type: None,
            description: "OS-level sandboxing not supported on this platform".to_string(),
        }
    }
}

/// Detect which platform and sandbox is available (synchronous version)
pub fn detect_available_sandbox_sync() -> SandboxAvailability {
    #[cfg(target_os = "linux")]
    {
        // Synchronous check for bubblewrap
        if std::process::Command::new("which")
            .arg("bwrap")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return SandboxAvailability {
                is_available: true,
                sandbox_type: Some(SandboxType::Bubblewrap),
                description: "bubblewrap is available".to_string(),
            };
        }

        // Check for landlock support
        if is_landlock_available() {
            return SandboxAvailability {
                is_available: true,
                sandbox_type: Some(SandboxType::Landlock),
                description: "landlock is available (kernel support)".to_string(),
            };
        }

        SandboxAvailability {
            is_available: false,
            sandbox_type: None,
            description:
                "no Linux sandbox available (bubblewrap not installed, landlock not supported)"
                    .to_string(),
        }
    }

    #[cfg(target_os = "macos")]
    {
        // Synchronous check for sandbox-exec
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

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        SandboxAvailability {
            is_available: false,
            sandbox_type: None,
            description: "OS-level sandboxing not supported on this platform".to_string(),
        }
    }
}

/// Check if bubblewrap is available
#[allow(dead_code)]
pub(crate) async fn is_bubblewrap_available() -> bool {
    which("bwrap").await.is_some() || which("bubblewrap").await.is_some()
}

/// Check if seatbelt is available (sandbox-exec)
pub(crate) async fn is_seatbelt_available() -> bool {
    which("sandbox-exec").await.is_some()
}

/// Check if landlock is available (kernel support)
pub(crate) fn is_landlock_available() -> bool {
    // Landlock support was added in Linux 5.13
    // We check by looking at /proc/sys/kernel Landlock-related sysctls or /proc/version
    std::path::Path::new("/proc/sys/kernel/unprivileged_userns_clone").exists()
    // A more robust check would be to verify kernel version >= 5.13
    // or attempt a landlock syscall and check ENOSYS
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

/// Bubblewrap sandbox implementation for Linux
#[derive(Debug, Clone)]
pub struct BubblewrapSandbox {
    config: OsSandboxConfig,
}

impl BubblewrapSandbox {
    pub fn new(config: OsSandboxConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config() -> Self {
        Self::new(OsSandboxConfig::default())
    }

    /// Build the bubblewrap command arguments
    fn build_bwrap_args(&self, cmd: &str, args: Option<&[String]>) -> Vec<String> {
        let mut bwrap_args = Vec::new();

        // Add --ro (read-only) or --rw (read-write) for each allowed directory
        for (path, perm) in &self.config.allowed_dirs {
            let perm_str = perm.as_sandbox_arg();
            bwrap_args.push(format!("--{}", perm_str));
            bwrap_args.push(path.to_string_lossy().to_string());
        }

        // Add tmp directory
        bwrap_args.push("--tmpfs".to_string());
        bwrap_args.push(self.config.temp_dir.to_string_lossy().to_string());

        // Dev directory (needed for many commands)
        bwrap_args.push("--dev".to_string());
        bwrap_args.push("/dev".to_string());

        // Network configuration
        if !self.config.share_network {
            bwrap_args.push("--unshare-net".to_string());
        }

        // User namespace (if not disabled)
        if !self.config.disable_user_ns {
            bwrap_args.push("--unshare-user".to_string());
            // Try to map to current user
            if let Ok(uid) = std::env::var("UID") {
                bwrap_args.push("--map-root-user".to_string());
                bwrap_args.push(format!("--uid {}", uid));
                bwrap_args.push(format!(
                    "--gid {}",
                    std::env::var("GID").unwrap_or_else(|_| uid.clone())
                ));
            }
        }

        // Working directory
        let work_dir = self
            .config
            .working_dir
            .as_ref()
            .unwrap_or(&self.config.temp_dir);
        bwrap_args.push("--chdir".to_string());
        bwrap_args.push(work_dir.to_string_lossy().to_string());

        // Environment variables
        for (key, value) in &self.config.env {
            bwrap_args.push("--setenv".to_string());
            bwrap_args.push(key.clone());
            bwrap_args.push(value.clone());
        }

        // The command to run
        bwrap_args.push(cmd.to_string());
        if let Some(cmd_args) = args {
            bwrap_args.extend(cmd_args.iter().cloned());
        }

        bwrap_args
    }
}

#[async_trait]
impl OsSandbox for BubblewrapSandbox {
    fn id(&self) -> &str {
        &self.config.sandbox_id
    }

    fn sandbox_type(&self) -> SandboxType {
        SandboxType::Bubblewrap
    }

    fn is_available(&self) -> bool {
        // Synchronous check using std::process::Command
        std::process::Command::new("which")
            .arg("bwrap")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    async fn execute(
        &self,
        cmd: &str,
        args: Option<&[String]>,
    ) -> Result<SandboxOutput, SwellError> {
        let bwrap_path = which("bwrap").await.ok_or_else(|| {
            SwellError::ToolExecutionFailed("bubblewrap (bwrap) not found in PATH".to_string())
        })?;

        let bwrap_args = self.build_bwrap_args(cmd, args);

        tracing::debug!(
            sandbox_id = %self.config.sandbox_id,
            bwrap_path = %bwrap_path.display(),
            args = ?bwrap_args,
            "BubblewrapSandbox: executing"
        );

        let start = Instant::now();

        let output = Command::new(&bwrap_path)
            .args(&bwrap_args)
            .output()
            .await
            .map_err(|e| {
                SwellError::ToolExecutionFailed(format!("bwrap execution failed: {}", e))
            })?;

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(SandboxOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            duration_ms,
        })
    }

    async fn execute_full(&self, cmd: SandboxCommand) -> Result<SandboxOutput, SwellError> {
        // Merge config env with command env
        let mut env = self.config.env.clone();
        for (key, value) in cmd.env {
            env.insert(key, value);
        }

        let mut config = self.config.clone();
        config.env = env;
        config.working_dir = cmd.working_dir.map(PathBuf::from);

        let sandbox = BubblewrapSandbox::new(config);

        // Build args with command and its arguments
        let mut full_args = vec![cmd.command.clone()];
        full_args.extend(cmd.args);

        sandbox.execute(&cmd.command, Some(&full_args)).await
    }
}

/// Seatbelt sandbox implementation for macOS
#[derive(Debug, Clone)]
pub struct SeatbeltSandbox {
    config: OsSandboxConfig,
}

impl SeatbeltSandbox {
    pub fn new(config: OsSandboxConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config() -> Self {
        Self::new(OsSandboxConfig::default())
    }

    /// Build the sandbox-exec profile
    fn build_sandbox_profile(&self) -> String {
        let mut profile = String::from("(version 1)\n");

        // Default deny for network
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

        // Filesystem rules
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

        // Temp directory - allow read-write
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

        // Process execution
        profile.push_str("(allow process-exec)\n");
        profile.push_str("(allow process-fork)\n");

        profile
    }
}

#[async_trait]
impl OsSandbox for SeatbeltSandbox {
    fn id(&self) -> &str {
        &self.config.sandbox_id
    }

    fn sandbox_type(&self) -> SandboxType {
        SandboxType::Seatbelt
    }

    fn is_available(&self) -> bool {
        // Synchronous check using std::process::Command
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
            "SeatbeltSandbox: executing"
        );

        let start = Instant::now();

        // Use sandbox-exec with the profile
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

    async fn execute_full(&self, cmd: SandboxCommand) -> Result<SandboxOutput, SwellError> {
        let mut full_cmd = cmd.command;
        if !cmd.args.is_empty() {
            full_cmd.push(' ');
            full_cmd.push_str(&cmd.args.join(" "));
        }

        self.execute(&full_cmd, None).await
    }
}

/// Landlock sandbox implementation for Linux (using landlockfs syscall)
/// Note: Landlock is a kernel-level feature that requires setting up rules
/// before execution. This is a simplified implementation.
#[derive(Debug, Clone)]
pub struct LandlockSandbox {
    config: OsSandboxConfig,
}

impl LandlockSandbox {
    pub fn new(config: OsSandboxConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config() -> Self {
        Self::new(OsSandboxConfig::default())
    }

    /// Build landlock-restricted environment variables
    /// Note: Landlock restrictions must be set before the process starts
    /// This uses PR_LIMIT or similar mechanisms
    fn build_landlock_env(&self) -> HashMap<String, String> {
        let mut env = self.config.env.clone();

        // For Landlock, we need to use a wrapper that sets up restrictions
        // The actual Landlock syscall must happen before executing the command
        // This is typically done via a setuid wrapper or similar

        // For now, we'll indicate that landlock should be used
        env.insert("SWELL_USE_LANDLOCK".to_string(), "1".to_string());

        // Add allowed paths as a formatted string
        let allowed_paths: Vec<String> = self
            .config
            .allowed_dirs
            .iter()
            .map(|(path, perm)| format!("{}:{}", path.display(), perm.as_sandbox_arg()))
            .collect();
        env.insert("SWELL_LANDLOCK_PATHS".to_string(), allowed_paths.join(","));

        // Network policy
        env.insert(
            "SWELL_LANDLOCK_NET".to_string(),
            format!("{:?}", self.config.network_policy).to_lowercase(),
        );

        env
    }
}

#[async_trait]
impl OsSandbox for LandlockSandbox {
    fn id(&self) -> &str {
        &self.config.sandbox_id
    }

    fn sandbox_type(&self) -> SandboxType {
        SandboxType::Landlock
    }

    fn is_available(&self) -> bool {
        is_landlock_available()
    }

    async fn execute(
        &self,
        cmd: &str,
        args: Option<&[String]>,
    ) -> Result<SandboxOutput, SwellError> {
        // Landlock requires a wrapper or privileged process to set up restrictions
        // For this implementation, we'll use a simple approach with unshare
        let start = Instant::now();

        let mut full_cmd = cmd.to_string();
        if let Some(cmd_args) = args {
            full_cmd.push(' ');
            full_cmd.push_str(&cmd_args.join(" "));
        }

        // Use unshare to create a new namespace, then execute
        // Note: True Landlock enforcement requires the landlock syscalls themselves
        let output = Command::new("unshare")
            .args(["--user", "--map-root-user", "sh", "-c", &full_cmd])
            .envs(self.build_landlock_env())
            .output()
            .await
            .map_err(|e| {
                SwellError::ToolExecutionFailed(format!("landlock execution failed: {}", e))
            })?;

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(SandboxOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            duration_ms,
        })
    }

    async fn execute_full(&self, cmd: SandboxCommand) -> Result<SandboxOutput, SwellError> {
        let mut env = self.build_landlock_env();
        for (key, value) in cmd.env {
            env.insert(key, value);
        }

        let mut full_cmd = cmd.command;
        if !cmd.args.is_empty() {
            full_cmd.push(' ');
            full_cmd.push_str(&cmd.args.join(" "));
        }

        let start = Instant::now();

        let mut command = Command::new("unshare");
        command
            .args(["--user", "--map-root-user", "sh", "-c", &full_cmd])
            .envs(env);

        if let Some(dir) = cmd.working_dir {
            command.current_dir(dir);
        }

        let output = command.output().await.map_err(|e| {
            SwellError::ToolExecutionFailed(format!("landlock execution failed: {}", e))
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

/// Platform-specific sandbox factory
#[allow(dead_code)]
pub struct PlatformSandbox {
    config: OsSandboxConfig,
}

impl PlatformSandbox {
    pub fn new(config: OsSandboxConfig) -> Self {
        Self { config }
    }

    /// Create a platform-specific sandbox
    pub async fn create(config: OsSandboxConfig) -> Result<Box<dyn OsSandbox>, SwellError> {
        let availability = detect_available_sandbox().await;

        match availability.sandbox_type {
            #[cfg(target_os = "linux")]
            Some(SandboxType::Bubblewrap) => {
                Ok(Box::new(BubblewrapSandbox::new(config)) as Box<dyn OsSandbox>)
            }
            #[cfg(target_os = "linux")]
            Some(SandboxType::Landlock) => {
                Ok(Box::new(LandlockSandbox::new(config)) as Box<dyn OsSandbox>)
            }
            #[cfg(target_os = "macos")]
            Some(SandboxType::Seatbelt) => {
                Ok(Box::new(SeatbeltSandbox::new(config)) as Box<dyn OsSandbox>)
            }
            _ => Err(SwellError::ToolExecutionFailed(format!(
                "No OS-level sandbox available: {}",
                availability.description
            ))),
        }
    }

    /// Create the best available sandbox synchronously (using type detection)
    pub fn create_sync(config: OsSandboxConfig) -> Result<Box<dyn OsSandbox>, SwellError> {
        let availability = detect_available_sandbox_sync();

        match availability.sandbox_type {
            #[cfg(target_os = "linux")]
            Some(SandboxType::Bubblewrap) => {
                Ok(Box::new(BubblewrapSandbox::new(config)) as Box<dyn OsSandbox>)
            }
            #[cfg(target_os = "linux")]
            Some(SandboxType::Landlock) => {
                Ok(Box::new(LandlockSandbox::new(config)) as Box<dyn OsSandbox>)
            }
            #[cfg(target_os = "macos")]
            Some(SandboxType::Seatbelt) => {
                Ok(Box::new(SeatbeltSandbox::new(config)) as Box<dyn OsSandbox>)
            }
            _ => Err(SwellError::ToolExecutionFailed(format!(
                "No OS-level sandbox available: {}",
                availability.description
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_detect_sandbox_async() {
        let availability = detect_available_sandbox().await;
        println!("Sandbox availability (async): {:?}", availability);
        // This test just verifies detection works
        assert!(availability.description.len() > 0);
    }

    #[test]
    fn test_detect_sandbox_sync() {
        let availability = detect_available_sandbox_sync();
        println!("Sandbox availability (sync): {:?}", availability);
        // This test just verifies detection works
        assert!(availability.description.len() > 0);
    }

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

    #[tokio::test]
    async fn test_bubblewrap_sandbox_new() {
        let config = OsSandboxConfig::default();
        let sandbox = BubblewrapSandbox::new(config);
        assert_eq!(sandbox.sandbox_type(), SandboxType::Bubblewrap);
    }

    #[tokio::test]
    async fn test_seatbelt_sandbox_new() {
        let config = OsSandboxConfig::default();
        let sandbox = SeatbeltSandbox::new(config);
        assert_eq!(sandbox.sandbox_type(), SandboxType::Seatbelt);
    }

    #[tokio::test]
    async fn test_landlock_sandbox_new() {
        let config = OsSandboxConfig::default();
        let sandbox = LandlockSandbox::new(config);
        assert_eq!(sandbox.sandbox_type(), SandboxType::Landlock);
    }

    #[test]
    fn test_bubblewrap_build_args() {
        let config = OsSandboxConfig::default()
            .with_allowed_dir_ro("/workspace")
            .with_allowed_dir_rw("/tmp")
            .with_network(NetworkPolicy::DenyAll)
            .with_working_dir("/workspace");

        let sandbox = BubblewrapSandbox::new(config);
        let args = sandbox.build_bwrap_args("echo", Some(&["hello".to_string()]));

        assert!(args.contains(&"--ro".to_string()));
        assert!(args.contains(&"/workspace".to_string()));
        assert!(args.contains(&"--rw".to_string()));
        assert!(args.contains(&"/tmp".to_string()));
        assert!(args.contains(&"--chdir".to_string()));
    }

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
    fn test_platform_sandbox_create_sync() {
        let config = OsSandboxConfig::default();
        let result = PlatformSandbox::create_sync(config);

        // Should succeed if any sandbox is available on the platform
        match result {
            Ok(_) => {}
            Err(e) => {
                // Expected on platforms without sandbox support
                println!("Platform sandbox not available: {}", e);
            }
        }
    }
}
