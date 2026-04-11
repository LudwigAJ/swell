//! gVisor (runsc) sandbox implementation for containerized tool execution.
//!
//! This module provides a gVisor-based sandbox that uses the `runsc` runtime to
//! execute commands with strong syscall isolation. gVisor intercepts syscalls in
//! user-space, providing container-like ergonomics with stronger isolation than
//! standard Docker containers.
//!
//! ## Key Features
//!
//! - **Syscall filtering**: gVisor's Sentry kernel only allows a restricted set of syscalls
//! - **User namespace isolation**: Processes run with distinct UID/GID mappings
//! - **No KVM required**: Works anywhere Docker runs, making it suitable for Kubernetes
//!
//! ## Usage
//!
//! ```rust,ignore
//! use swell_sandbox::GvisorSandbox;
//!
//! let sandbox = GvisorSandbox::new(GvisorConfig::default());
//! sandbox.start().await?;
//! let output = sandbox.execute(cmd).await?;
//! sandbox.stop().await?;
//! ```

use async_trait::async_trait;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Instant;
use swell_core::{Sandbox, SandboxCommand, SandboxOutput, SwellError};
use tokio::process::Command;

/// Configuration for a gVisor sandbox instance
#[derive(Debug, Clone)]
pub struct GvisorConfig {
    /// Unique identifier for this sandbox
    pub sandbox_id: String,
    /// Docker image to use for the sandbox
    pub image: String,
    /// Working directory inside the container
    pub working_dir: Option<String>,
    /// Enable user namespace isolation
    pub enable_user_namespace: bool,
    /// Enable seccomp filtering (default for gVisor)
    pub enable_seccomp: bool,
    /// Additional environment variables
    pub env: std::collections::HashMap<String, String>,
    /// Execution timeout in seconds
    pub timeout_secs: u64,
    /// Path to the runsc binary (defaults to /usr/bin/runsc)
    pub runsc_path: String,
    /// Network mode (none, bridge, host)
    pub network_mode: GvisorNetworkMode,
}

impl Default for GvisorConfig {
    fn default() -> Self {
        Self {
            sandbox_id: uuid::Uuid::new_v4().to_string(),
            image: "ubuntu:22.04".to_string(),
            working_dir: None,
            enable_user_namespace: true,
            enable_seccomp: true,
            env: std::collections::HashMap::new(),
            timeout_secs: 300,
            runsc_path: "/usr/bin/runsc".to_string(),
            network_mode: GvisorNetworkMode::None,
        }
    }
}

/// Network mode for gVisor sandbox
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GvisorNetworkMode {
    /// No network access
    #[default]
    None,
    /// Bridge network (default Docker networking)
    Bridge,
    /// Use host network directly
    Host,
}

impl std::fmt::Display for GvisorNetworkMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GvisorNetworkMode::None => write!(f, "none"),
            GvisorNetworkMode::Bridge => write!(f, "bridge"),
            GvisorNetworkMode::Host => write!(f, "host"),
        }
    }
}

/// A gVisor-based sandbox implementation.
///
/// This sandbox uses Docker with the `runsc` runtime to provide:
/// - User-space syscall interception via gVisor's Sentry
/// - User namespace isolation for UID/GID mapping
/// - Seccomp filtering for additional security
///
/// ## Requirements
///
/// - Docker daemon running
/// - gVisor `runsc` runtime installed and registered with Docker
/// - Proper permissions to access Docker socket
///
/// ## Example
///
/// ```rust,ignore
/// let config = GvisorConfig {
///     sandbox_id: "my-sandbox".to_string(),
///     image: "ubuntu:22.04".to_string(),
///     enable_user_namespace: true,
///     ..Default::default()
/// };
/// let sandbox = GvisorSandbox::new(config);
/// ```
pub struct GvisorSandbox {
    config: GvisorConfig,
    container_id: Mutex<Option<String>>,
}

impl GvisorSandbox {
    /// Create a new GvisorSandbox with the given configuration
    pub fn new(config: GvisorConfig) -> Self {
        Self {
            config,
            container_id: Mutex::new(None),
        }
    }

    /// Create a new GvisorSandbox with simple parameters
    pub fn with_params(sandbox_id: String, image: String) -> Self {
        Self {
            config: GvisorConfig {
                sandbox_id,
                image,
                ..Default::default()
            },
            container_id: Mutex::new(None),
        }
    }

    /// Get the configuration for this sandbox
    pub fn config(&self) -> &GvisorConfig {
        &self.config
    }

    /// Get the container ID if the sandbox is running
    pub fn container_id(&self) -> Option<String> {
        self.container_id.lock().unwrap().clone()
    }

    /// Check if gVisor runsc is available on this system
    pub async fn is_runsc_available() -> Result<bool, SwellError> {
        let output = Command::new("docker")
            .args(["info"])
            .output()
            .await
            .map_err(|e| {
                SwellError::ToolExecutionFailed(format!("Failed to check Docker: {}", e))
            })?;

        if !output.status.success() {
            return Ok(false);
        }

        // Check if runsc runtime is registered
        let output = Command::new("docker")
            .args(["info", "--format", "{{.Runtimes}}"])
            .output()
            .await
            .map_err(|e| {
                SwellError::ToolExecutionFailed(format!("Failed to check runtimes: {}", e))
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.contains("runsc"))
    }

    /// Build Docker run flags for gVisor
    fn build_docker_flags(&self) -> Vec<String> {
        let mut flags = vec!["--runtime".to_string(), "runsc".to_string()];

        // User namespace isolation
        if self.config.enable_user_namespace {
            flags.push("--user".to_string());
            flags.push("65534".to_string()); // Nobody user
        }

        // Network mode
        flags.push("--network".to_string());
        flags.push(self.config.network_mode.to_string());

        // Auto-remove container when it exits
        flags.push("--rm".to_string());

        // Disable resource limits for now (can be added later)
        // flags.push("--memory".to_string());
        // flags.push("512m".to_string());

        flags
    }

    /// Execute a command inside the gVisor container using docker exec
    async fn exec_in_container(&self, cmd: &SandboxCommand) -> Result<SandboxOutput, SwellError> {
        let container_id = {
            let guard = self.container_id.lock().unwrap();
            guard.clone().ok_or_else(|| SwellError::ToolExecutionFailed("Container not running".to_string()))?
        };

        let start = Instant::now();

        // Build docker exec command
        let mut args = vec!["exec".to_string(), container_id.to_string()];

        // Add environment variables
        for (key, value) in &cmd.env {
            args.push("-e".to_string());
            args.push(format!("{}={}", key, value));
        }

        // Add working directory if specified
        if let Some(ref dir) = cmd.working_dir {
            args.push("-w".to_string());
            args.push(dir.clone());
        }

        // Add the command
        args.push(cmd.command.clone());
        args.extend(cmd.args.clone());

        let mut command = Command::new("docker");
        command.args(&args);

        // Set stdout/stderr capture
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());

        tracing::debug!(
            sandbox_id = %self.config.sandbox_id,
            container_id = %container_id,
            command = %cmd.command,
            args = ?cmd.args,
            "GvisorSandbox: executing command in container"
        );

        let output = command
            .output()
            .await
            .map_err(|e| SwellError::ToolExecutionFailed(format!("docker exec failed: {}", e)))?;

        let duration_ms = start.elapsed().as_millis() as u64;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        Ok(SandboxOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout,
            stderr,
            duration_ms,
        })
    }

    /// Write a file to the container via docker cp
    async fn write_file_to_container(
        &self,
        container_id: &str,
        path: &str,
        content: &[u8],
    ) -> Result<(), SwellError> {
        // Create a temporary file on host
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("gvisor_upload_{}", uuid::Uuid::new_v4()));

        tokio::fs::write(&temp_file, content).await.map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to write temp file: {}", e))
        })?;

        // Copy to container
        let container_path = format!("{}:{}", container_id, path);
        let output = Command::new("docker")
            .args([
                "cp",
                temp_file.to_string_lossy().as_ref(),
                container_path.as_str(),
            ])
            .output()
            .await
            .map_err(|e| SwellError::ToolExecutionFailed(format!("docker cp failed: {}", e)))?;

        // Clean up temp file
        let _ = tokio::fs::remove_file(&temp_file).await;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SwellError::ToolExecutionFailed(format!(
                "docker cp failed: {}",
                stderr
            )));
        }

        Ok(())
    }

    /// Read a file from the container via docker cp
    async fn read_file_from_container(
        &self,
        container_id: &str,
        path: &str,
    ) -> Result<Vec<u8>, SwellError> {
        // Copy file to temporary location
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("gvisor_download_{}", uuid::Uuid::new_v4()));

        let container_path = format!("{}:{}", container_id, path);
        let output = Command::new("docker")
            .args([
                "cp",
                container_path.as_str(),
                temp_file.to_string_lossy().as_ref(),
            ])
            .output()
            .await
            .map_err(|e| SwellError::ToolExecutionFailed(format!("docker cp failed: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SwellError::ToolExecutionFailed(format!(
                "docker cp failed: {}",
                stderr
            )));
        }

        let content = tokio::fs::read(&temp_file).await.map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to read temp file: {}", e))
        })?;

        // Clean up temp file
        let _ = tokio::fs::remove_file(&temp_file).await;

        Ok(content)
    }
}

#[async_trait]
impl Sandbox for GvisorSandbox {
    fn id(&self) -> &str {
        &self.config.sandbox_id
    }

    async fn start(&self) -> Result<(), SwellError> {
        tracing::info!(
            sandbox_id = %self.config.sandbox_id,
            image = %self.config.image,
            enable_user_namespace = %self.config.enable_user_namespace,
            enable_seccomp = %self.config.enable_seccomp,
            "GvisorSandbox: starting container"
        );

        // Check if Docker is available
        let output = Command::new("docker")
            .args(["info"])
            .output()
            .await
            .map_err(|e| SwellError::ToolExecutionFailed(format!("Docker not available: {}", e)))?;

        if !output.status.success() {
            return Err(SwellError::ToolExecutionFailed(
                "Docker daemon not running".to_string(),
            ));
        }

        // Build docker run command
        let mut args = vec!["run".to_string()];

        // Add gVisor flags
        args.extend(self.build_docker_flags());

        // Add environment variables
        for (key, value) in &self.config.env {
            args.push("-e".to_string());
            args.push(format!("{}={}", key, value));
        }

        // Add working directory if specified
        if let Some(ref dir) = self.config.working_dir {
            args.push("-w".to_string());
            args.push(dir.clone());
        }

        // Detached mode
        args.push("-d".to_string());

        // Container image
        args.push(self.config.image.clone());

        // Use sleep infinity as placeholder command to keep container running
        args.push("sleep".to_string());
        args.push("infinity".to_string());

        tracing::debug!(
            sandbox_id = %self.config.sandbox_id,
            "GvisorSandbox: running docker with args: {:?}",
            args
        );

        let output = Command::new("docker")
            .args(&args)
            .output()
            .await
            .map_err(|e| SwellError::ToolExecutionFailed(format!("docker run failed: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SwellError::ToolExecutionFailed(format!(
                "docker run failed with exit code {:?}: {}",
                output.status.code(),
                stderr
            )));
        }

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

        tracing::info!(
            sandbox_id = %self.config.sandbox_id,
            container_id = %container_id,
            "GvisorSandbox: container started"
        );

        // Store the container_id for subsequent operations
        let mut guard = self.container_id.lock().unwrap();
        *guard = Some(container_id);
        Ok(())
    }

    async fn stop(&self) -> Result<(), SwellError> {
        tracing::info!(
            sandbox_id = %self.config.sandbox_id,
            "GvisorSandbox: stopping container"
        );

        let container_id = {
            let guard = self.container_id.lock().unwrap();
            guard.clone()
        };

        if let Some(container_id) = container_id {
            let output = Command::new("docker")
                .args(["kill", &container_id])
                .output()
                .await
                .map_err(|e| {
                    SwellError::ToolExecutionFailed(format!("docker kill failed: {}", e))
                })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::warn!(
                    sandbox_id = %self.config.sandbox_id,
                    container_id = %container_id,
                    error = %stderr,
                    "GvisorSandbox: container may not have been killed"
                );
            }
        }

        Ok(())
    }

    async fn execute(&self, cmd: SandboxCommand) -> Result<SandboxOutput, SwellError> {
        tracing::info!(
            sandbox_id = %self.config.sandbox_id,
            command = %cmd.command,
            args = ?cmd.args,
            "GvisorSandbox: execute (using docker exec)"
        );

        self.exec_in_container(&cmd).await
    }

    async fn write_file(&self, path: &str, content: &[u8]) -> Result<(), SwellError> {
        tracing::info!(
            sandbox_id = %self.config.sandbox_id,
            path = %path,
            size = %content.len(),
            "GvisorSandbox: write_file"
        );

        let container_id = {
            let guard = self.container_id.lock().unwrap();
            guard.clone().ok_or_else(|| SwellError::ToolExecutionFailed("Container not running".to_string()))?
        };

        self.write_file_to_container(&container_id, path, content)
            .await
    }

    async fn read_file(&self, path: &str) -> Result<Vec<u8>, SwellError> {
        tracing::info!(
            sandbox_id = %self.config.sandbox_id,
            path = %path,
            "GvisorSandbox: read_file"
        );

        let container_id = {
            let guard = self.container_id.lock().unwrap();
            guard.clone().ok_or_else(|| SwellError::ToolExecutionFailed("Container not running".to_string()))?
        };

        self.read_file_from_container(&container_id, path).await
    }

    async fn is_running(&self) -> bool {
        let container_id = {
            let guard = self.container_id.lock().unwrap();
            guard.clone()
        };
        if let Some(container_id) = container_id {
            let output = Command::new("docker")
                .args(["inspect", "--format", "{{.State.Running}}", &container_id])
                .output()
                .await;

            if let Ok(output) = output {
                let stdout = String::from_utf8_lossy(&output.stdout);
                return stdout.trim() == "true";
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_gvisor_sandbox_new() {
        let config = GvisorConfig {
            sandbox_id: "test-gvisor-1".to_string(),
            image: "ubuntu:22.04".to_string(),
            ..Default::default()
        };
        let sandbox = GvisorSandbox::new(config);
        assert_eq!(sandbox.id(), "test-gvisor-1");
    }

    #[tokio::test]
    async fn test_gvisor_sandbox_with_params() {
        let sandbox =
            GvisorSandbox::with_params("test-gvisor-2".to_string(), "alpine:latest".to_string());
        assert_eq!(sandbox.id(), "test-gvisor-2");
        assert_eq!(sandbox.config().image, "alpine:latest");
    }

    #[test]
    fn test_gvisor_config_default() {
        let config = GvisorConfig::default();
        assert!(config.enable_user_namespace);
        assert!(config.enable_seccomp);
        assert_eq!(config.network_mode, GvisorNetworkMode::None);
    }

    #[test]
    fn test_gvisor_network_mode_display() {
        assert_eq!(GvisorNetworkMode::None.to_string(), "none");
        assert_eq!(GvisorNetworkMode::Bridge.to_string(), "bridge");
        assert_eq!(GvisorNetworkMode::Host.to_string(), "host");
    }

    #[tokio::test]
    async fn test_gvisor_sandbox_build_docker_flags() {
        let config = GvisorConfig {
            sandbox_id: "test".to_string(),
            enable_user_namespace: true,
            network_mode: GvisorNetworkMode::None,
            ..Default::default()
        };
        let sandbox = GvisorSandbox::new(config);
        let flags = sandbox.build_docker_flags();

        assert!(flags.contains(&"--runtime".to_string()));
        assert!(flags.contains(&"runsc".to_string()));
        assert!(flags.contains(&"--user".to_string()));
        assert!(flags.contains(&"--network".to_string()));
        assert!(flags.contains(&"none".to_string()));
    }

    #[tokio::test]
    async fn test_gvisor_sandbox_no_user_namespace() {
        let config = GvisorConfig {
            sandbox_id: "test".to_string(),
            enable_user_namespace: false,
            ..Default::default()
        };
        let sandbox = GvisorSandbox::new(config);
        let flags = sandbox.build_docker_flags();

        // Should not contain --user flag when disabled
        assert!(!flags.contains(&"--user".to_string()));
    }
}
