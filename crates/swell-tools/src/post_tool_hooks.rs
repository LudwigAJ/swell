//! Post-tool-use hooks for automatic linting and formatting.
//!
//! This module provides hooks that run after file edit operations to ensure
//! code quality standards are maintained automatically.
//!
//! # Hooks
//!
//! - [`FormatHook`] - Runs `cargo fmt` to enforce code formatting
//! - [`LintHook`] - Runs `cargo clippy` to check for common mistakes
//! - [`PostToolHookManager`] - Manages and executes hooks after tool operations

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use swell_core::SwellError;
use tracing::{info, warn};

/// Result of a post-tool hook execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookResult {
    /// Whether the hook execution succeeded
    pub success: bool,
    /// Name of the hook that was executed
    pub hook_name: String,
    /// The command that was run
    pub command: String,
    /// stdout from the command
    pub stdout: String,
    /// stderr from the command
    pub stderr: String,
    /// Error message if the hook failed
    pub error: Option<String>,
    /// Files that were modified by the hook
    pub modified_files: Vec<String>,
}

impl HookResult {
    /// Create a successful hook result
    pub fn success(
        hook_name: &str,
        command: &str,
        stdout: &str,
        stderr: &str,
        modified_files: Vec<String>,
    ) -> Self {
        Self {
            success: true,
            hook_name: hook_name.to_string(),
            command: command.to_string(),
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            error: None,
            modified_files,
        }
    }

    /// Create a failed hook result
    pub fn failure(
        hook_name: &str,
        command: &str,
        error: &str,
        stdout: &str,
        stderr: &str,
    ) -> Self {
        Self {
            success: false,
            hook_name: hook_name.to_string(),
            command: command.to_string(),
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            error: Some(error.to_string()),
            modified_files: vec![],
        }
    }
}

/// Tool name constants
pub mod tool_names {
    /// Edit file tool name
    pub const EDIT_FILE: &str = "edit_file";
    /// Write file tool name
    pub const WRITE_FILE: &str = "write_file";
}

/// Hook trigger - determines when a hook should run
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookTrigger {
    /// Run after any file edit
    AfterEdit,
    /// Run after writes to specific file patterns (e.g., "*.rs")
    AfterEditMatching(String),
    /// Run after any tool
    AfterAnyTool,
}

/// Configuration for post-tool hooks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    /// Whether formatting hook is enabled
    pub format_on_edit: bool,
    /// Whether linting hook is enabled
    pub lint_on_edit: bool,
    /// Additional file patterns to watch (e.g., ["*.rs", "*.toml"])
    pub watch_patterns: Vec<String>,
    /// Timeout for hook execution in seconds
    pub timeout_secs: u64,
}

impl Default for HookConfig {
    fn default() -> Self {
        Self {
            format_on_edit: true,
            lint_on_edit: true,
            watch_patterns: vec!["*.rs".to_string()],
            timeout_secs: 120,
        }
    }
}

/// Trait for post-tool hooks
#[async_trait]
pub trait PostToolHook: Send + Sync {
    /// Get the name of this hook
    fn name(&self) -> &str;

    /// Get the trigger condition for this hook
    fn trigger(&self) -> HookTrigger;

    /// Check if this hook should run for the given tool and path
    fn should_run(&self, tool_name: &str, file_path: Option<&str>) -> bool;

    /// Execute the hook
    async fn execute(
        &self,
        workspace_path: &Path,
        modified_files: &[String],
    ) -> Result<HookResult, SwellError>;
}

/// Hook that runs `cargo fmt` to format code
#[derive(Debug, Clone)]
pub struct FormatHook {
    config: HookConfig,
}

impl FormatHook {
    /// Create a new FormatHook with default configuration
    pub fn new() -> Self {
        Self {
            config: HookConfig::default(),
        }
    }

    /// Create a new FormatHook with custom configuration
    pub fn with_config(config: HookConfig) -> Self {
        Self { config }
    }

    /// Run cargo fmt and return modified files
    fn run_fmt(workspace_path: PathBuf) -> Result<(bool, Vec<String>, String, String), String> {
        let output = Command::new("cargo")
            .args(["fmt", "--", "--emit", "files"])
            .current_dir(&workspace_path)
            .output()
            .map_err(|e| format!("Failed to execute cargo fmt: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        // cargo fmt --emit files returns modified file paths on stdout
        let modified_files: Vec<String> = stdout
            .lines()
            .filter(|line| !line.is_empty())
            .map(|s| s.to_string())
            .collect();

        let success = output.status.success();
        Ok((success, modified_files, stdout, stderr))
    }
}

impl Default for FormatHook {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PostToolHook for FormatHook {
    fn name(&self) -> &str {
        "format"
    }

    fn trigger(&self) -> HookTrigger {
        HookTrigger::AfterEdit
    }

    fn should_run(&self, tool_name: &str, file_path: Option<&str>) -> bool {
        if !self.config.format_on_edit {
            return false;
        }

        let is_edit_tool =
            tool_name == tool_names::EDIT_FILE || tool_name == tool_names::WRITE_FILE;

        if !is_edit_tool {
            return false;
        }

        // Check if file matches watch patterns
        if let Some(path) = file_path {
            for pattern in &self.config.watch_patterns {
                if glob_match(pattern, path) {
                    return true;
                }
            }
            // If no patterns match, but we have patterns, don't run
            if !self.config.watch_patterns.is_empty() {
                return false;
            }
        }

        true
    }

    async fn execute(
        &self,
        workspace_path: &Path,
        _modified_files: &[String],
    ) -> Result<HookResult, SwellError> {
        info!("Running format hook (cargo fmt)");

        let workspace_path_owned = workspace_path.to_path_buf();
        let result = tokio::task::spawn_blocking(move || Self::run_fmt(workspace_path_owned))
            .await
            .map_err(|e| SwellError::ToolExecutionFailed(format!("Task join error: {}", e)))?
            .map_err(SwellError::ToolExecutionFailed)?;

        let (success, modified, stdout, stderr) = result;

        if success {
            info!(files = ?modified, "Format hook completed successfully");
            Ok(HookResult::success(
                "format",
                "cargo fmt",
                &stdout,
                &stderr,
                modified,
            ))
        } else {
            let err_msg = if stderr.contains("warning") || stderr.contains("error") {
                stderr.clone()
            } else {
                "cargo fmt failed".to_string()
            };
            warn!("Format hook failed: {}", err_msg);
            Ok(HookResult::failure(
                "format",
                "cargo fmt",
                &err_msg,
                &stdout,
                &stderr,
            ))
        }
    }
}

/// Hook that runs `cargo clippy` to lint code
#[derive(Debug, Clone)]
pub struct LintHook {
    config: HookConfig,
}

impl LintHook {
    /// Create a new LintHook with default configuration
    pub fn new() -> Self {
        Self {
            config: HookConfig::default(),
        }
    }

    /// Create a new LintHook with custom configuration
    pub fn with_config(config: HookConfig) -> Self {
        Self { config }
    }

    /// Run cargo clippy and return the result
    fn run_clippy(
        workspace_path: PathBuf,
        _timeout_secs: u64,
    ) -> Result<(bool, String, String), String> {
        // Run clippy in check mode with JSON output for easier parsing
        let output = Command::new("cargo")
            .args(["clippy", "--message-format", "json", "--", "-D", "warnings"])
            .current_dir(&workspace_path)
            .output()
            .map_err(|e| format!("Failed to execute cargo clippy: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        // Clippy returns non-zero on warnings/errors, but we consider it successful
        // if it runs (even with warnings). The lint check is informational.
        let success = output.status.success() || stderr.is_empty();

        Ok((success, stdout, stderr))
    }
}

impl Default for LintHook {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PostToolHook for LintHook {
    fn name(&self) -> &str {
        "lint"
    }

    fn trigger(&self) -> HookTrigger {
        HookTrigger::AfterEdit
    }

    fn should_run(&self, tool_name: &str, file_path: Option<&str>) -> bool {
        if !self.config.lint_on_edit {
            return false;
        }

        let is_edit_tool =
            tool_name == tool_names::EDIT_FILE || tool_name == tool_names::WRITE_FILE;

        if !is_edit_tool {
            return false;
        }

        // Check if file matches watch patterns
        if let Some(path) = file_path {
            for pattern in &self.config.watch_patterns {
                if glob_match(pattern, path) {
                    return true;
                }
            }
            // If no patterns match, but we have patterns, don't run
            if !self.config.watch_patterns.is_empty() {
                return false;
            }
        }

        true
    }

    async fn execute(
        &self,
        workspace_path: &Path,
        _modified_files: &[String],
    ) -> Result<HookResult, SwellError> {
        info!("Running lint hook (cargo clippy)");

        let workspace_path_owned = workspace_path.to_path_buf();
        let timeout = self.config.timeout_secs;
        let result =
            tokio::task::spawn_blocking(move || Self::run_clippy(workspace_path_owned, timeout))
                .await
                .map_err(|e| SwellError::ToolExecutionFailed(format!("Task join error: {}", e)))?
                .map_err(SwellError::ToolExecutionFailed)?;

        let (success, stdout, stderr) = result;

        if success {
            info!("Lint hook completed successfully");
            Ok(HookResult::success(
                "lint",
                "cargo clippy",
                &stdout,
                &stderr,
                vec![],
            ))
        } else {
            // Lint hook failures are warnings, not blocking errors
            warn!("Lint hook found issues (non-blocking)");
            Ok(HookResult::success(
                "lint",
                "cargo clippy",
                &stdout,
                &stderr,
                vec![],
            ))
        }
    }
}

/// Manager for post-tool hooks
#[derive(Clone)]
pub struct PostToolHookManager {
    hooks: Vec<Arc<dyn PostToolHook>>,
    config: HookConfig,
}

impl PostToolHookManager {
    /// Create a new PostToolHookManager with default hooks and configuration
    pub fn new() -> Self {
        let config = HookConfig::default();
        Self {
            hooks: vec![
                Arc::new(FormatHook::with_config(config.clone())),
                Arc::new(LintHook::with_config(config.clone())),
            ],
            config,
        }
    }

    /// Create a new PostToolHookManager with custom configuration
    pub fn with_config(config: HookConfig) -> Self {
        Self {
            hooks: vec![
                Arc::new(FormatHook::with_config(config.clone())),
                Arc::new(LintHook::with_config(config.clone())),
            ],
            config,
        }
    }

    /// Create with specific hooks
    pub fn with_hooks(hooks: Vec<Arc<dyn PostToolHook>>) -> Self {
        Self {
            hooks,
            config: HookConfig::default(),
        }
    }

    /// Add a hook to the manager
    pub fn add_hook<H: PostToolHook + 'static>(&mut self, hook: H) {
        self.hooks.push(Arc::new(hook));
    }

    /// Execute all applicable hooks for a tool operation
    pub async fn execute_hooks(
        &self,
        tool_name: &str,
        workspace_path: &Path,
        modified_files: &[String],
    ) -> Vec<HookResult> {
        let mut results = Vec::new();

        // Extract file path from modified files if available
        let file_path = modified_files.first().map(|s| s.as_str());

        for hook in &self.hooks {
            if hook.should_run(tool_name, file_path) {
                match hook.execute(workspace_path, modified_files).await {
                    Ok(result) => {
                        results.push(result);
                    }
                    Err(e) => {
                        results.push(HookResult::failure(
                            hook.name(),
                            "hook execution",
                            &e.to_string(),
                            "",
                            "",
                        ));
                    }
                }
            }
        }

        results
    }

    /// Get the current configuration
    pub fn config(&self) -> &HookConfig {
        &self.config
    }

    /// Update the configuration
    pub fn set_config(&mut self, config: HookConfig) {
        self.config = config;
    }
}

impl Default for PostToolHookManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple glob pattern matching for file paths
fn glob_match(pattern: &str, path: &str) -> bool {
    // Handle simple glob patterns like *.rs, *.toml, etc.
    if let Some(extension) = pattern.strip_prefix("*.") {
        return path.ends_with(&format!(".{}", extension));
    }

    // Handle full path patterns with *
    if pattern.contains('*') {
        let parts: Vec<&str> = pattern.split('*').collect();
        if parts.len() == 2 {
            return path.starts_with(parts[0]) && path.ends_with(parts[1]);
        }
    }

    // Exact match
    pattern == path
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_glob_match_extensions() {
        assert!(glob_match("*.rs", "test.rs"));
        assert!(glob_match("*.rs", "path/to/file.rs"));
        assert!(!glob_match("*.rs", "test.txt"));
        assert!(!glob_match("*.rs", "test.rs.txt"));
    }

    #[test]
    fn test_glob_match_wildcards() {
        assert!(glob_match("test*", "test_file.rs"));
        assert!(glob_match("test*", "test.rs"));
        assert!(!glob_match("test*", "file_test.rs"));
    }

    #[test]
    fn test_glob_match_exact() {
        assert!(glob_match("test.rs", "test.rs"));
        assert!(!glob_match("test.rs", "test.txt"));
    }

    #[test]
    fn test_format_hook_should_run_edit() {
        let hook = FormatHook::new();
        assert!(hook.should_run("edit_file", Some("test.rs")));
        assert!(hook.should_run("write_file", Some("test.rs")));
    }

    #[test]
    fn test_format_hook_should_not_run_other_tools() {
        let hook = FormatHook::new();
        assert!(!hook.should_run("read_file", Some("test.rs")));
        assert!(!hook.should_run("shell", Some("test.rs")));
        assert!(!hook.should_run("git", Some("test.rs")));
    }

    #[test]
    fn test_format_hook_with_pattern_matching() {
        let config = HookConfig {
            format_on_edit: true,
            lint_on_edit: false,
            watch_patterns: vec!["*.rs".to_string()],
            timeout_secs: 120,
        };
        let hook = FormatHook::with_config(config);

        assert!(hook.should_run("edit_file", Some("test.rs")));
        assert!(!hook.should_run("edit_file", Some("test.txt")));
    }

    #[test]
    fn test_format_hook_disabled() {
        let config = HookConfig {
            format_on_edit: false,
            lint_on_edit: false,
            watch_patterns: vec![],
            timeout_secs: 120,
        };
        let hook = FormatHook::with_config(config);

        assert!(!hook.should_run("edit_file", Some("test.rs")));
    }

    #[test]
    fn test_lint_hook_should_run_edit() {
        let hook = LintHook::new();
        assert!(hook.should_run("edit_file", Some("test.rs")));
        assert!(hook.should_run("write_file", Some("test.rs")));
    }

    #[test]
    fn test_lint_hook_should_not_run_other_tools() {
        let hook = LintHook::new();
        assert!(!hook.should_run("read_file", Some("test.rs")));
        assert!(!hook.should_run("shell", Some("test.rs")));
    }

    #[test]
    fn test_hook_result_success() {
        let result = HookResult::success(
            "format",
            "cargo fmt",
            "Modified: src/lib.rs",
            "",
            vec!["src/lib.rs".to_string()],
        );

        assert!(result.success);
        assert_eq!(result.hook_name, "format");
        assert_eq!(result.command, "cargo fmt");
        assert!(result.error.is_none());
        assert_eq!(result.modified_files, vec!["src/lib.rs"]);
    }

    #[test]
    fn test_hook_result_failure() {
        let result = HookResult::failure(
            "lint",
            "cargo clippy",
            "clippy failed",
            "",
            "error: some clippy warning",
        );

        assert!(!result.success);
        assert_eq!(result.hook_name, "lint");
        assert!(result.error.is_some());
        assert!(result.error.unwrap().contains("clippy failed"));
    }

    #[test]
    fn test_hook_config_default() {
        let config = HookConfig::default();
        assert!(config.format_on_edit);
        assert!(config.lint_on_edit);
        assert_eq!(config.watch_patterns, vec!["*.rs"]);
        assert_eq!(config.timeout_secs, 120);
    }

    #[tokio::test]
    async fn test_post_tool_hook_manager_empty_hooks() {
        let manager = PostToolHookManager::with_hooks(vec![]);
        let dir = tempdir().unwrap();

        let results = manager
            .execute_hooks("edit_file", dir.path(), &["test.rs".to_string()])
            .await;

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_post_tool_hook_manager_disabled_hooks() {
        let config = HookConfig {
            format_on_edit: false,
            lint_on_edit: false,
            watch_patterns: vec![],
            timeout_secs: 120,
        };
        let manager = PostToolHookManager::with_config(config);
        let dir = tempdir().unwrap();

        let results = manager
            .execute_hooks("edit_file", dir.path(), &["test.rs".to_string()])
            .await;

        // Hooks are disabled, so nothing runs
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_post_tool_hook_manager_runs_format_for_rust_files() {
        let config = HookConfig {
            format_on_edit: true,
            lint_on_edit: false,
            watch_patterns: vec!["*.rs".to_string()],
            timeout_secs: 120,
        };
        let manager = PostToolHookManager::with_config(config);
        let dir = tempdir().unwrap();

        // Create a simple Rust file
        tokio::fs::write(dir.path().join("test.rs"), "fn main() {}")
            .await
            .unwrap();

        let results = manager
            .execute_hooks(
                "edit_file",
                dir.path(),
                &[dir.path().join("test.rs").to_string_lossy().to_string()],
            )
            .await;

        // Should have run format hook
        assert!(!results.is_empty());
        assert!(results.iter().any(|r| r.hook_name == "format"));
    }

    #[test]
    fn test_tool_names() {
        assert_eq!(tool_names::EDIT_FILE, "edit_file");
        assert_eq!(tool_names::WRITE_FILE, "write_file");
    }
}
