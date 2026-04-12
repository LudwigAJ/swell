//! Swell agent hooks system - similar to Claude Code hooks.
//!
//! Hooks are configured in `.swell/hooks.json` and can execute custom scripts
//! or commands at various points in the agent lifecycle.
//!
//! # Supported Hooks
//!
//! | Hook | Description |
//! |------|-------------|
//! | `stop` | Called when stop is requested |
//! | `post_tool_use` | Called after each tool completes |
//! | `pre_tool_use` | Called before each tool runs |
//! | `on_tool_error` | Called when a tool fails |
//! | `pre_commit` | Called before git commit |
//! | `post_commit` | Called after git commit |
//! | `on_branch_switch` | Called when switching branches |
//! | `on_step_complete` | Called after each agent step |
//! | `on_agent_start` | Called when agent starts |
//! | `on_agent_complete` | Called when agent completes |
//!
//! # Configuration
//!
//! Hooks can be commands or scripts:
//!
//! ```json
//! {
//!   "hooks": {
//!     "post_tool_use": {
//!       "enabled": true,
//!       "command": "cargo clippy --message-format json",
//!       "script": null,
//!       "timeout_ms": 5000
//!     }
////!   }
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use swell_core::ToolOutput;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Hook event types
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookEvent {
    Stop,
    PostToolUse,
    PreToolUse,
    OnToolError,
    PreCommit,
    PostCommit,
    OnBranchSwitch,
    OnStepComplete,
    OnAgentStart,
    OnAgentComplete,
}

impl HookEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            HookEvent::Stop => "stop",
            HookEvent::PostToolUse => "post_tool_use",
            HookEvent::PreToolUse => "pre_tool_use",
            HookEvent::OnToolError => "on_tool_error",
            HookEvent::PreCommit => "pre_commit",
            HookEvent::PostCommit => "post_commit",
            HookEvent::OnBranchSwitch => "on_branch_switch",
            HookEvent::OnStepComplete => "on_step_complete",
            HookEvent::OnAgentStart => "on_agent_start",
            HookEvent::OnAgentComplete => "on_agent_complete",
        }
    }
}

impl std::fmt::Display for HookEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A single hook configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HookConfig {
    pub description: Option<String>,
    pub enabled: bool,
    pub command: Option<String>,
    pub script: Option<String>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub async_exec: bool,
    #[serde(default)]
    pub continue_on_error: bool,
}

fn default_timeout_ms() -> u64 {
    30000
}

/// Hooks configuration file format
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HooksConfig {
    #[serde(default)]
    pub description: Option<String>,
    pub hooks: HashMap<String, HookConfig>,
    #[serde(default)]
    pub defaults: HooksDefaults,
}

/// Default settings for hooks
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HooksDefaults {
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub continue_on_error: bool,
    #[serde(default)]
    pub async_exec: bool,
}

impl Default for HooksDefaults {
    fn default() -> Self {
        Self {
            timeout_ms: 30000,
            continue_on_error: false,
            async_exec: false,
        }
    }
}

/// Result of hook execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookResult {
    pub event: String,
    pub hook_name: String,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub error: Option<String>,
    pub exit_code: Option<i32>,
}

impl HookResult {
    pub fn success(
        event: &str,
        hook_name: &str,
        stdout: &str,
        stderr: &str,
        duration_ms: u64,
    ) -> Self {
        Self {
            event: event.to_string(),
            hook_name: hook_name.to_string(),
            success: true,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            duration_ms,
            error: None,
            exit_code: Some(0),
        }
    }

    pub fn failure(
        event: &str,
        hook_name: &str,
        error: &str,
        stdout: &str,
        stderr: &str,
        duration_ms: u64,
        exit_code: Option<i32>,
    ) -> Self {
        Self {
            event: event.to_string(),
            hook_name: hook_name.to_string(),
            success: false,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            duration_ms,
            error: Some(error.to_string()),
            exit_code,
        }
    }
}

/// Context passed to hook execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookContext {
    pub event: String,
    pub tool_name: Option<String>,
    pub tool_args: Option<serde_json::Value>,
    pub tool_output: Option<serde_json::Value>,
    pub error: Option<String>,
    pub agent_id: Option<String>,
    pub task_id: Option<String>,
    pub workspace_path: Option<String>,
    pub metadata: HashMap<String, String>,
}

impl HookContext {
    pub fn for_tool_use(tool_name: &str, args: serde_json::Value) -> Self {
        Self {
            event: HookEvent::PostToolUse.as_str().to_string(),
            tool_name: Some(tool_name.to_string()),
            tool_args: Some(args),
            tool_output: None,
            error: None,
            agent_id: None,
            task_id: None,
            workspace_path: None,
            metadata: HashMap::new(),
        }
    }

    pub fn for_tool_error(tool_name: &str, args: serde_json::Value, error: &str) -> Self {
        Self {
            event: HookEvent::OnToolError.as_str().to_string(),
            tool_name: Some(tool_name.to_string()),
            tool_args: Some(args),
            tool_output: None,
            error: Some(error.to_string()),
            agent_id: None,
            task_id: None,
            workspace_path: None,
            metadata: HashMap::new(),
        }
    }

    pub fn with_output(mut self, output: ToolOutput) -> Self {
        self.tool_output = Some(serde_json::to_value(&output).unwrap_or_default());
        self
    }

    pub fn with_agent_id(mut self, agent_id: &str) -> Self {
        self.agent_id = Some(agent_id.to_string());
        self
    }

    pub fn with_task_id(mut self, task_id: &str) -> Self {
        self.task_id = Some(task_id.to_string());
        self
    }

    pub fn with_workspace(mut self, path: &Path) -> Self {
        self.workspace_path = Some(path.to_string_lossy().to_string());
        self
    }
}

/// Hooks manager - loads config and executes hooks
#[derive(Debug, Clone)]
pub struct HooksManager {
    config: Arc<RwLock<Option<HooksConfig>>>,
    config_path: PathBuf,
    workspace_path: PathBuf,
    enabled: bool,
}

impl HooksManager {
    /// Create a new hooks manager
    pub fn new(workspace_path: PathBuf) -> Self {
        Self {
            config: Arc::new(RwLock::new(None)),
            config_path: workspace_path.join(".swell").join("hooks.json"),
            workspace_path,
            enabled: true,
        }
    }

    /// Create with explicit config path
    pub fn with_config_path(workspace_path: PathBuf, config_path: PathBuf) -> Self {
        Self {
            config: Arc::new(RwLock::new(None)),
            config_path: config_path,
            workspace_path,
            enabled: true,
        }
    }

    /// Load hooks configuration from file
    pub async fn load_config(&self) -> Result<(), HooksError> {
        if !self.config_path.exists() {
            debug!("No hooks.json found at {:?}", self.config_path);
            return Ok(());
        }

        let content = tokio::fs::read_to_string(&self.config_path)
            .await
            .map_err(|e| HooksError::ConfigReadError(e.to_string()))?;

        let config: HooksConfig = serde_json::from_str(&content)
            .map_err(|e| HooksError::ConfigParseError(e.to_string()))?;

        let mut cfg = self.config.write().await;
        *cfg = Some(config);

        info!(path = ?self.config_path, "Loaded hooks configuration");
        Ok(())
    }

    /// Reload configuration from file
    pub async fn reload(&self) -> Result<(), HooksError> {
        self.load_config().await
    }

    /// Enable or disable hooks
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check if hooks are enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Execute a hook by event type
    pub async fn execute(&self, event: HookEvent, context: HookContext) -> Vec<HookResult> {
        if !self.enabled {
            return vec![];
        }

        let config = self.config.read().await;
        let config = match config.as_ref() {
            Some(c) => c,
            None => return vec![],
        };

        let hook_name = event.as_str();
        let Some(hook_config) = config.hooks.get(hook_name) else {
            return vec![];
        };

        if !hook_config.enabled {
            debug!(hook = %hook_name, "Hook is disabled");
            return vec![];
        }

        self.execute_hook(hook_config, event, context).await
    }

    /// Execute a single hook
    async fn execute_hook(
        &self,
        hook_config: &HookConfig,
        event: HookEvent,
        context: HookContext,
    ) -> Vec<HookResult> {
        let hook_name = event.as_str();
        let timeout = Duration::from_millis(hook_config.timeout_ms);

        // Execute command hook
        if let Some(command) = &hook_config.command {
            if command.is_empty() {
                return vec![];
            }
            return vec![
                self.run_command(command, hook_config, hook_name, &context, timeout)
                    .await,
            ];
        }

        // Execute script hook
        if let Some(script) = &hook_config.script {
            if script.is_empty() {
                return vec![];
            }
            return vec![
                self.run_script(script, hook_config, hook_name, &context, timeout)
                    .await,
            ];
        }

        vec![]
    }

    /// Run a command hook
    async fn run_command(
        &self,
        command: &str,
        _hook_config: &HookConfig,
        hook_name: &str,
        context: &HookContext,
        _timeout: Duration,
    ) -> HookResult {
        let start = Instant::now();
        let command = command.to_string();
        let workspace_path = self.workspace_path.clone();

        // Prepare environment variables from context
        let mut env_vars: HashMap<String, String> = HashMap::new();
        if let Some(tool_name) = &context.tool_name {
            env_vars.insert("SWELL_TOOL_NAME".to_string(), tool_name.clone());
        }
        if let Some(error) = &context.error {
            env_vars.insert("SWELL_ERROR".to_string(), error.clone());
        }
        if let Some(agent_id) = &context.agent_id {
            env_vars.insert("SWELL_AGENT_ID".to_string(), agent_id.clone());
        }
        if let Some(task_id) = &context.task_id {
            env_vars.insert("SWELL_TASK_ID".to_string(), task_id.clone());
        }

        let result = tokio::task::spawn_blocking(move || {
            let mut cmd = Command::new("sh");
            cmd.arg("-c")
                .arg(&command)
                .current_dir(&workspace_path)
                .envs(&env_vars);

            cmd.output()
        })
        .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let exit_code = output.status.code();
                let success = output.status.success();

                if success {
                    debug!(hook = %hook_name, duration_ms = %duration_ms, "Hook executed successfully");
                } else {
                    warn!(
                        hook = %hook_name,
                        exit_code = ?exit_code,
                        duration_ms = %duration_ms,
                        "Hook failed"
                    );
                }

                HookResult {
                    event: hook_name.to_string(),
                    hook_name: hook_name.to_string(),
                    success,
                    stdout,
                    stderr,
                    duration_ms,
                    error: None,
                    exit_code,
                }
            }
            Ok(Err(e)) => {
                error!(hook = %hook_name, error = %e, "Hook command failed");
                HookResult::failure(
                    hook_name,
                    hook_name,
                    &format!("Command failed: {}", e),
                    "",
                    "",
                    duration_ms,
                    None,
                )
            }
            Err(e) => {
                error!(hook = %hook_name, error = %e, "Hook execution failed");
                HookResult::failure(
                    hook_name,
                    hook_name,
                    &e.to_string(),
                    "",
                    "",
                    duration_ms,
                    None,
                )
            }
        }
    }

    /// Run a script hook
    async fn run_script(
        &self,
        script: &str,
        _hook_config: &HookConfig,
        hook_name: &str,
        context: &HookContext,
        _timeout: Duration,
    ) -> HookResult {
        let script_path: PathBuf = if script.starts_with('/') {
            PathBuf::from(script)
        } else {
            self.workspace_path.join(script)
        };

        if !script_path.exists() {
            return HookResult::failure(
                hook_name,
                hook_name,
                &format!("Script not found: {:?}", script_path),
                "",
                "",
                0,
                None,
            );
        }

        let workspace_path = self.workspace_path.clone();

        // Prepare environment variables
        let mut env_vars: HashMap<String, String> = HashMap::new();
        if let Some(tool_name) = &context.tool_name {
            env_vars.insert("SWELL_TOOL_NAME".to_string(), tool_name.clone());
        }
        if let Some(error) = &context.error {
            env_vars.insert("SWELL_ERROR".to_string(), error.clone());
        }
        env_vars.insert(
            "SWELL_CONTEXT".to_string(),
            serde_json::to_string(context).unwrap_or_default(),
        );

        let start = Instant::now();

        let result = tokio::task::spawn_blocking(move || {
            let mut cmd = Command::new(&script_path);
            cmd.current_dir(&workspace_path).envs(&env_vars);

            cmd.output()
        })
        .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let exit_code = output.status.code();
                let success = output.status.success();

                HookResult {
                    event: hook_name.to_string(),
                    hook_name: hook_name.to_string(),
                    success,
                    stdout,
                    stderr,
                    duration_ms,
                    error: None,
                    exit_code,
                }
            }
            Ok(Err(e)) => HookResult::failure(
                hook_name,
                hook_name,
                &e.to_string(),
                "",
                "",
                duration_ms,
                None,
            ),
            Err(e) => HookResult::failure(
                hook_name,
                hook_name,
                &e.to_string(),
                "",
                "",
                duration_ms,
                None,
            ),
        }
    }

    /// Get the current configuration
    pub async fn get_config(&self) -> Option<HooksConfig> {
        self.config.read().await.clone()
    }

    /// Check if any hooks are defined
    pub async fn has_hooks(&self) -> bool {
        self.config.read().await.is_some()
    }
}

/// Hooks error types
#[derive(Debug, thiserror::Error)]
pub enum HooksError {
    #[error("Failed to read hooks config: {0}")]
    ConfigReadError(String),

    #[error("Failed to parse hooks config: {0}")]
    ConfigParseError(String),

    #[error("Hook execution failed: {0}")]
    ExecutionError(String),

    #[error("Hook timeout after {0}ms")]
    Timeout(u64),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hook_event_as_str() {
        assert_eq!(HookEvent::Stop.as_str(), "stop");
        assert_eq!(HookEvent::PostToolUse.as_str(), "post_tool_use");
        assert_eq!(HookEvent::OnToolError.as_str(), "on_tool_error");
    }

    #[test]
    fn test_hook_context_for_tool_use() {
        let ctx = HookContext::for_tool_use("read_file", serde_json::json!({"path": "/tmp/test"}));
        assert_eq!(ctx.event, "post_tool_use");
        assert_eq!(ctx.tool_name, Some("read_file".to_string()));
        assert!(ctx.error.is_none());
    }

    #[test]
    fn test_hook_context_for_tool_error() {
        let ctx = HookContext::for_tool_error(
            "read_file",
            serde_json::json!({"path": "/tmp/test"}),
            "File not found",
        );
        assert_eq!(ctx.event, "on_tool_error");
        assert_eq!(ctx.error, Some("File not found".to_string()));
    }

    #[test]
    fn test_hook_result_success() {
        let result = HookResult::success("post_tool_use", "post_tool_use", "output", "", 100);
        assert!(result.success);
        assert_eq!(result.exit_code, Some(0));
    }

    #[test]
    fn test_hook_result_failure() {
        let result = HookResult::failure(
            "post_tool_use",
            "post_tool_use",
            "Command failed",
            "",
            "error output",
            100,
            Some(1),
        );
        assert!(!result.success);
        assert_eq!(result.error, Some("Command failed".to_string()));
    }

    #[test]
    fn test_hooks_manager_default_disabled_when_no_config() {
        let manager = HooksManager::new(PathBuf::from("/tmp"));
        assert!(manager.is_enabled());
    }

    #[tokio::test]
    async fn test_hooks_manager_no_config() {
        let manager = HooksManager::new(PathBuf::from("/tmp/nonexistent"));
        manager.load_config().await.ok();
        assert!(!manager.has_hooks().await);
    }
}
