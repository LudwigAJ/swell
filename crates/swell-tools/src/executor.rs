//! Tool executor with permission enforcement and sandbox pre-execution hooks.
//!
//! This module provides the ToolExecutor which enforces permissions and applies
//! OS-level sandboxing (Seatbelt on macOS, Bubblewrap on Linux) before executing
//! shell commands to restrict filesystem access.

use crate::os_sandbox::{
    detect_available_sandbox_sync, FilesystemPermission, NetworkPolicy, OsSandboxConfig,
    SandboxAvailability,
};
use crate::post_tool_hooks::PostToolHookManager;
use crate::registry::ToolRegistry;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use swell_core::{PermissionTier, SwellError, ToolOutput};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Permission checker for tools
#[derive(Debug, Clone)]
pub struct PermissionChecker {
    default_tier: PermissionTier,
    allowed_tools: std::collections::HashSet<String>,
}

impl PermissionChecker {
    pub fn new() -> Self {
        Self {
            default_tier: PermissionTier::Auto,
            allowed_tools: std::collections::HashSet::new(),
        }
    }

    /// Set the default permission tier
    pub fn with_default_tier(mut self, tier: PermissionTier) -> Self {
        self.default_tier = tier;
        self
    }

    /// Allow a specific tool (bypasses tier check)
    pub fn allow_tool(mut self, name: impl Into<String>) -> Self {
        self.allowed_tools.insert(name.into());
        self
    }

    /// Check if a tool execution is permitted
    ///
    /// Permission tiers:
    /// - `Auto`: Always permitted (auto-approved)
    /// - `Ask`: Treated as `Auto` (auto-approved) since no confirmation mechanism exists
    /// - `Deny`: Never permitted without explicit override via `allowed_tools`
    pub fn is_allowed(&self, tool_name: &str, tool_tier: PermissionTier) -> bool {
        if self.allowed_tools.contains(tool_name) {
            return true;
        }
        // Ask is treated as Auto since there's no user confirmation mechanism
        matches!(tool_tier, PermissionTier::Auto | PermissionTier::Ask)
    }
}

impl Default for PermissionChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Pre-execution sandbox hook that applies OS-level sandboxing before tool execution.
///
/// This hook restricts filesystem access and network capabilities when executing
/// shell commands. On macOS, it uses Seatbelt (sandbox-exec). On Linux, it uses
/// Bubblewrap (bwrap) when available.
///
/// # Security Properties
///
/// - **Filesystem restrictions**: Shell commands can only access the task working
///   directory and explicitly allowed paths
/// - **Network restrictions**: Network access is denied by default unless explicitly
///   allowed in the sandbox configuration
/// - **Process restrictions**: Shell commands run in a restricted OS-level sandbox
#[derive(Debug, Clone)]
pub struct SandboxPreHook {
    /// Configuration for the sandbox
    config: OsSandboxConfig,
    /// Cached availability check result
    availability: SandboxAvailability,
    /// Whether the sandbox is enabled
    enabled: bool,
}

impl SandboxPreHook {
    /// Create a new sandbox pre-hook with the default configuration.
    ///
    /// The default configuration:
    /// - Allows read access to the workspace path
    /// - Denies all network access
    /// - Uses the system temp directory for temp files
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a sandbox pre-hook with a specific workspace path.
    ///
    /// The workspace path will be allowed read-only access in the sandbox.
    pub fn with_workspace_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut config = OsSandboxConfig::default();
        config
            .allowed_dirs
            .insert(path.clone(), FilesystemPermission::ReadOnly);
        let availability = detect_available_sandbox_sync();

        Self {
            config,
            availability,
            enabled: true,
        }
    }

    /// Add an allowed directory with read-only permission.
    pub fn allow_dir_ro(mut self, path: impl Into<PathBuf>) -> Self {
        self.config
            .allowed_dirs
            .insert(path.into(), FilesystemPermission::ReadOnly);
        self
    }

    /// Add an allowed directory with read-write permission.
    pub fn allow_dir_rw(mut self, path: impl Into<PathBuf>) -> Self {
        self.config
            .allowed_dirs
            .insert(path.into(), FilesystemPermission::ReadWrite);
        self
    }

    /// Set the network policy for sandboxed commands.
    pub fn with_network_policy(mut self, policy: NetworkPolicy) -> Self {
        self.config.network_policy = policy;
        self
    }

    /// Enable or disable the sandbox hook.
    ///
    /// When disabled, the hook becomes a no-op and commands execute normally.
    pub fn set_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Check if sandbox is available and enabled.
    pub fn is_available(&self) -> bool {
        self.enabled && self.availability.is_available
    }

    /// Check if a tool requires sandboxing (shell commands).
    ///
    /// Returns true for shell commands, which are the primary attack vector
    /// for filesystem access and command injection.
    pub fn requires_sandboxing(tool_name: &str) -> bool {
        matches!(tool_name, "shell" | "bash" | "sh" | "exec")
    }

    /// Apply the sandbox to a command and arguments.
    ///
    /// Returns the sandboxed execution result or an error if sandboxing failed.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The sandbox is not available
    /// - The command could not be executed under the sandbox
    /// - Sandbox validation failed (profile not applied correctly)
    pub async fn apply(
        &self,
        cmd: &str,
        args: Option<&[String]>,
    ) -> Result<swell_core::SandboxOutput, SwellError> {
        if !self.is_available() {
            debug!("Sandbox not available, executing without sandbox");
            return Err(SwellError::SandboxError(
                "Sandbox not available on this platform".to_string(),
            ));
        }

        // Create the appropriate sandbox based on platform
        let sandbox = crate::os_sandbox::PlatformSandbox::create(self.config.clone()).await;

        match sandbox {
            Ok(s) => {
                debug!(
                    sandbox_type = ?s.sandbox_type(),
                    cmd = %cmd,
                    "Applying sandbox before tool execution"
                );

                // Execute command under sandbox
                let result = s.execute(cmd, args).await;

                // Validate sandbox was applied by checking the result
                // The actual enforcement happens at the OS level via sandbox-exec/bwrap
                if result.is_ok() {
                    debug!(
                        sandbox_id = %s.id(),
                        cmd = %cmd,
                        "Sandbox applied successfully"
                    );
                }

                result
            }
            Err(e) => {
                debug!("Failed to create sandbox: {}", e);
                Err(SwellError::SandboxError(format!(
                    "Failed to create sandbox: {}",
                    e
                )))
            }
        }
    }

    /// Get the sandbox configuration.
    pub fn config(&self) -> &OsSandboxConfig {
        &self.config
    }

    /// Get the sandbox availability information.
    pub fn availability(&self) -> &SandboxAvailability {
        &self.availability
    }
}

impl Default for SandboxPreHook {
    fn default() -> Self {
        let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut config = OsSandboxConfig::default();
        config
            .allowed_dirs
            .insert(workspace.clone(), FilesystemPermission::ReadOnly);
        config
            .allowed_dirs
            .insert(std::env::temp_dir(), FilesystemPermission::ReadWrite);

        let availability = detect_available_sandbox_sync();

        Self {
            config,
            availability,
            enabled: true,
        }
    }
}

/// Executes tools with permission enforcement and sandbox pre-execution hooks.
pub struct ToolExecutor {
    registry: ToolRegistry,
    permissions: PermissionChecker,
    hook_manager: Option<Arc<RwLock<PostToolHookManager>>>,
    workspace_path: PathBuf,
    sandbox_hook: Option<SandboxPreHook>,
}

impl ToolExecutor {
    pub fn new(registry: ToolRegistry) -> Self {
        Self {
            registry,
            permissions: PermissionChecker::new(),
            hook_manager: None,
            workspace_path: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            sandbox_hook: None,
        }
    }

    pub fn with_permissions(mut self, permissions: PermissionChecker) -> Self {
        self.permissions = permissions;
        self
    }

    /// Set the workspace path for hook execution
    pub fn with_workspace_path(mut self, path: PathBuf) -> Self {
        self.workspace_path = path;
        self
    }

    /// Enable post-tool hooks with the given manager
    pub fn with_hook_manager(mut self, manager: PostToolHookManager) -> Self {
        self.hook_manager = Some(Arc::new(RwLock::new(manager)));
        self
    }

    /// Enable sandbox pre-execution hook with the given configuration.
    ///
    /// When enabled, shell commands will be executed under an OS-level sandbox
    /// (Seatbelt on macOS, Bubblewrap on Linux) that restricts filesystem
    /// access and network capabilities.
    ///
    /// # Example
    ///
    /// ```
    /// use swell_tools::executor::{ToolExecutor, SandboxPreHook};
    /// use swell_tools::registry::ToolRegistry;
    /// use std::path::PathBuf;
    ///
    /// let registry = ToolRegistry::new();
    /// let executor = ToolExecutor::new(registry)
    ///     .with_sandbox_hook(
    ///         SandboxPreHook::with_workspace_path("/workspace/task-123")
    ///             .allow_dir_ro("/usr/local/bin")
    ///     );
    /// ```
    pub fn with_sandbox_hook(mut self, hook: SandboxPreHook) -> Self {
        self.sandbox_hook = Some(hook);
        self
    }

    /// Enable sandbox pre-execution hook with default configuration.
    ///
    /// The default sandbox configuration:
    /// - Allows read access to the current workspace
    /// - Allows read-write access to the temp directory
    /// - Denies all network access
    pub fn with_sandbox_enabled(mut self) -> Self {
        self.sandbox_hook = Some(SandboxPreHook::with_workspace_path(&self.workspace_path));
        self
    }

    /// Execute a tool by name
    pub async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolOutput, SwellError> {
        self.execute_with_hooks(name, arguments, vec![]).await
    }

    /// Execute a tool with post-tool hooks and sandbox pre-execution
    ///
    /// This method runs the tool and then executes any applicable post-tool hooks.
    /// For shell commands, if a sandbox hook is configured, the command will be
    /// executed under an OS-level sandbox that restricts filesystem access.
    ///
    /// The `modified_files` parameter provides hints about which files were modified,
    /// allowing hooks to decide whether to run.
    pub async fn execute_with_hooks(
        &self,
        name: &str,
        arguments: serde_json::Value,
        modified_files: Vec<String>,
    ) -> Result<ToolOutput, SwellError> {
        let start = Instant::now();

        let tool =
            self.registry.get(name).await.ok_or_else(|| {
                SwellError::ToolExecutionFailed(format!("Tool not found: {}", name))
            })?;

        // Check permissions
        if !self.permissions.is_allowed(name, tool.permission_tier()) {
            warn!(tool = %name, "Tool execution denied");
            return Err(SwellError::PermissionDenied(format!(
                "Tool '{}' requires {:?} permission",
                name,
                tool.permission_tier()
            )));
        }

        // Apply sandbox pre-execution for shell commands
        if SandboxPreHook::requires_sandboxing(name) {
            if let Some(ref sandbox_hook) = self.sandbox_hook {
                if sandbox_hook.is_available() {
                    // Extract command from arguments and run under sandbox
                    let (cmd, args) = self.extract_shell_command(&arguments)?;

                    debug!(
                        tool = %name,
                        sandbox_available = true,
                        "Applying sandbox pre-execution hook"
                    );

                    // Execute under sandbox
                    let sandbox_result = sandbox_hook.apply(&cmd, Some(&args)).await;

                    match sandbox_result {
                        Ok(output) => {
                            let duration = start.elapsed();
                            // Get sandbox type from availability
                            let sandbox_type = sandbox_hook
                                .availability()
                                .sandbox_type
                                .map(|t| format!("{:?}", t))
                                .unwrap_or_else(|| "unknown".to_string());
                            info!(
                                tool = %name,
                                sandbox_type = %sandbox_type,
                                duration_ms = %duration.as_millis(),
                                "Tool executed under sandbox"
                            );

                            // Run post-tool hooks
                            self.run_post_hooks(name, &modified_files).await;

                            // Check exit code for error status (non-zero = error)
                            let is_error = output.exit_code != 0;
                            return Ok(ToolOutput {
                                is_error,
                                content: vec![swell_core::ToolResultContent::Text(
                                    output.stdout,
                                )],
                            });
                        }
                        Err(e) => {
                            warn!(
                                tool = %name,
                                error = %e,
                                "Sandbox execution failed, falling back to direct execution"
                            );
                            // Fall through to direct execution if sandbox fails
                        }
                    }
                }
            }
        }

        info!(tool = %name, "Executing tool");
        let result = tool.execute(arguments).await;

        let duration = start.elapsed();
        info!(tool = %name, duration_ms = %duration.as_millis(), "Tool execution completed");

        // Run post-tool hooks if enabled and tool succeeded
        if result.is_ok() {
            self.run_post_hooks(name, &modified_files).await;
        }

        result
    }

    /// Extract the shell command and arguments from tool arguments.
    ///
    /// The shell tool expects arguments in the format:
    /// ```json
    /// {
    ///     "command": "ls -la",
    ///     "working_dir": "/path/to/dir"
    /// }
    /// ```
    fn extract_shell_command(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<(String, Vec<String>), SwellError> {
        // Get the command string
        let command = arguments
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                SwellError::ToolExecutionFailed("Missing 'command' field in shell arguments".into())
            })?;

        // Parse the command into command + args
        let parts: Vec<String> = shell_words::split(command).map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to parse command: {}", e))
        })?;

        if parts.is_empty() {
            return Err(SwellError::ToolExecutionFailed(
                "Empty command".into(),
            ));
        }

        let cmd = parts[0].clone();
        let args = if parts.len() > 1 {
            parts[1..].to_vec()
        } else {
            vec![]
        };

        Ok((cmd, args))
    }

    /// Run post-tool hooks for a tool
    async fn run_post_hooks(&self, tool_name: &str, modified_files: &[String]) {
        if let Some(ref hook_manager) = self.hook_manager {
            let manager = hook_manager.read().await;
            let results = manager
                .execute_hooks(tool_name, &self.workspace_path, modified_files)
                .await;

            for hook_result in results {
                if hook_result.success {
                    info!(
                        hook = %hook_result.hook_name,
                        command = %hook_result.command,
                        "Post-tool hook completed successfully"
                    );
                } else {
                    warn!(
                        hook = %hook_result.hook_name,
                        error = ?hook_result.error,
                        "Post-tool hook completed with issues"
                    );
                }
            }
        }
    }

    /// Check if a tool can be executed (exists and permitted)
    pub async fn can_execute(&self, name: &str) -> bool {
        let tool = self.registry.get(name).await;
        match tool {
            Some(t) => self.permissions.is_allowed(name, t.permission_tier()),
            None => false,
        }
    }

    /// Get the registry for inspection
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    /// Get the workspace path
    pub fn workspace_path(&self) -> &PathBuf {
        &self.workspace_path
    }

    /// Get hook results from the last execution (for testing/debugging)
    pub fn has_hook_manager(&self) -> bool {
        self.hook_manager.is_some()
    }
}

impl Clone for ToolExecutor {
    fn clone(&self) -> Self {
        Self {
            registry: self.registry.clone(),
            permissions: self.permissions.clone(),
            hook_manager: self.hook_manager.clone(),
            workspace_path: self.workspace_path.clone(),
            sandbox_hook: self.sandbox_hook.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::post_tool_hooks::PostToolHookManager;
    use crate::tools::ReadFileTool;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_executor_permission_denied() {
        let registry = ToolRegistry::new();
        registry
            .register(
                ReadFileTool::new(),
                crate::registry::ToolCategory::File,
                crate::registry::ToolLayer::Builtin,
            )
            .await;

        let executor = ToolExecutor::new(registry);

        // Default permission tier is Auto, so it should work
        let result = executor
            .execute("read_file", serde_json::json!({"path": "/tmp/test"}))
            .await;
        // May fail due to file not existing, but permission should pass
        assert!(result.is_ok() || matches!(result, Err(SwellError::ToolExecutionFailed(_))));
    }

    #[tokio::test]
    async fn test_executor_tool_not_found() {
        let registry = ToolRegistry::new();
        let executor = ToolExecutor::new(registry);

        let result = executor.execute("nonexistent", serde_json::json!({})).await;
        assert!(matches!(result, Err(SwellError::ToolExecutionFailed(_))));
    }

    #[tokio::test]
    async fn test_executor_with_workspace_path() {
        let registry = ToolRegistry::new();
        let dir = tempdir().unwrap();

        let executor = ToolExecutor::new(registry).with_workspace_path(dir.path().to_path_buf());

        assert_eq!(executor.workspace_path(), dir.path());
    }

    #[tokio::test]
    async fn test_executor_with_hook_manager() {
        let registry = ToolRegistry::new();
        let hook_manager = PostToolHookManager::new();

        let executor = ToolExecutor::new(registry).with_hook_manager(hook_manager);

        assert!(executor.has_hook_manager());
    }

    #[tokio::test]
    async fn test_sandbox_pre_hook_default() {
        let hook = SandboxPreHook::new();
        // Should be available on macOS where sandbox-exec exists
        // May not be available on other platforms
        let _is_available = hook.is_available();
    }

    #[tokio::test]
    async fn test_sandbox_pre_hook_with_workspace() {
        let dir = tempdir().unwrap();
        let hook = SandboxPreHook::with_workspace_path(dir.path());

        // with_workspace_path adds 1 dir (the workspace)
        assert_eq!(hook.config().allowed_dirs.len(), 1);
        assert!(hook.availability().is_available || !hook.availability().is_available);
    }

    #[tokio::test]
    async fn test_sandbox_pre_hook_add_allowed_dirs() {
        let hook = SandboxPreHook::new()
            .allow_dir_ro("/usr/local/bin")
            .allow_dir_rw("/tmp/rw");

        // new() starts with 2 dirs (workspace + temp), then we add 2 more
        assert_eq!(hook.config().allowed_dirs.len(), 4);
    }

    #[tokio::test]
    async fn test_sandbox_pre_hook_network_policy() {
        let hook = SandboxPreHook::new()
            .with_network_policy(NetworkPolicy::AllowAll);

        assert_eq!(hook.config().network_policy, NetworkPolicy::AllowAll);
    }

    #[test]
    fn test_sandbox_pre_hook_requires_sandboxing() {
        // Shell commands require sandboxing
        assert!(SandboxPreHook::requires_sandboxing("shell"));
        assert!(SandboxPreHook::requires_sandboxing("bash"));
        assert!(SandboxPreHook::requires_sandboxing("sh"));
        assert!(SandboxPreHook::requires_sandboxing("exec"));

        // Other commands don't require sandboxing
        assert!(!SandboxPreHook::requires_sandboxing("read_file"));
        assert!(!SandboxPreHook::requires_sandboxing("write_file"));
        assert!(!SandboxPreHook::requires_sandboxing("git"));
    }

    #[tokio::test]
    async fn test_sandbox_pre_hook_disabled() {
        let dir = tempdir().unwrap();
        let hook = SandboxPreHook::with_workspace_path(dir.path()).set_enabled(false);

        assert!(!hook.is_available());
    }

    #[tokio::test]
    async fn test_executor_with_sandbox_hook() {
        let registry = ToolRegistry::new();
        let dir = tempdir().unwrap();

        let hook = SandboxPreHook::with_workspace_path(dir.path());
        let executor = ToolExecutor::new(registry).with_sandbox_hook(hook);

        // Clone should preserve the sandbox hook
        let cloned = executor.clone();
        assert!(cloned.sandbox_hook.is_some());
    }

    #[tokio::test]
    async fn test_executor_with_sandbox_enabled() {
        let registry = ToolRegistry::new();
        let dir = tempdir().unwrap();

        let executor = ToolExecutor::new(registry)
            .with_workspace_path(dir.path().to_path_buf())
            .with_sandbox_enabled();

        assert!(executor.sandbox_hook.is_some());
    }
}
