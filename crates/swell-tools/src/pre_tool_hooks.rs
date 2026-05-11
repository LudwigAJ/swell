//! Pre-tool hooks for deterministic deny checks before tool execution.
//!
//! These hooks run before permission checks, sandbox routing, or the tool body.

use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;
use swell_core::{SwellError, ToolOutput, ToolResultContent};

/// Result of a pre-tool hook evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreToolDecision {
    /// Continue evaluating subsequent hooks and then execute the tool.
    Allow,
    /// Stop immediately and return the reason as a tool error.
    Deny(String),
}

impl PreToolDecision {
    pub fn into_tool_output(self) -> Option<ToolOutput> {
        match self {
            Self::Allow => None,
            Self::Deny(reason) => Some(ToolOutput {
                is_error: true,
                content: vec![ToolResultContent::Error(format!(
                    "Pre-tool hook denied execution: {reason}"
                ))],
            }),
        }
    }
}

/// Hook evaluated before a tool is executed.
#[async_trait]
pub trait PreToolHook: Send + Sync {
    fn name(&self) -> &str;

    async fn evaluate(
        &self,
        tool_name: &str,
        arguments: &serde_json::Value,
        workspace_path: &Path,
    ) -> Result<PreToolDecision, SwellError>;
}

/// Denies shell/git command strings matching configured dangerous patterns.
#[derive(Debug, Clone)]
pub struct CommandDenyHook {
    denied_patterns: Vec<String>,
}

impl CommandDenyHook {
    pub fn new(denied_patterns: Vec<String>) -> Self {
        Self { denied_patterns }
    }

    pub fn default_patterns() -> Vec<String> {
        vec![
            "rm -rf /".to_string(),
            "rm -rf /*".to_string(),
            "DROP DATABASE".to_string(),
            "--force".to_string(),
            "reset --hard".to_string(),
        ]
    }

    fn command_for(tool_name: &str, arguments: &serde_json::Value) -> Option<String> {
        match tool_name {
            "shell" | "bash" | "sh" | "exec" => arguments
                .get("command")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            "git" => {
                let operation = arguments
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let args = arguments
                    .get("args")
                    .and_then(|v| v.as_array())
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|v| v.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                Some(format!("git {operation} {args}"))
            }
            _ => None,
        }
    }

    fn matches_denied_pattern(&self, command: &str) -> Option<&str> {
        let normalized_command = command.to_ascii_lowercase();
        self.denied_patterns.iter().find_map(|pattern| {
            let normalized_pattern = pattern.to_ascii_lowercase();
            if normalized_command.contains(&normalized_pattern) {
                Some(pattern.as_str())
            } else {
                None
            }
        })
    }
}

#[async_trait]
impl PreToolHook for CommandDenyHook {
    fn name(&self) -> &str {
        "command_deny"
    }

    async fn evaluate(
        &self,
        tool_name: &str,
        arguments: &serde_json::Value,
        _workspace_path: &Path,
    ) -> Result<PreToolDecision, SwellError> {
        let Some(command) = Self::command_for(tool_name, arguments) else {
            return Ok(PreToolDecision::Allow);
        };

        if let Some(pattern) = self.matches_denied_pattern(&command) {
            return Ok(PreToolDecision::Deny(format!(
                "tool `{tool_name}` command matched denied pattern `{pattern}`"
            )));
        }

        Ok(PreToolDecision::Allow)
    }
}

/// Runs pre-tool hooks in order and stops on the first denial.
#[derive(Clone)]
pub struct PreToolHookManager {
    hooks: Vec<Arc<dyn PreToolHook>>,
}

impl PreToolHookManager {
    pub fn new() -> Self {
        Self {
            hooks: vec![Arc::new(CommandDenyHook::new(
                CommandDenyHook::default_patterns(),
            ))],
        }
    }

    pub fn with_hooks(hooks: Vec<Arc<dyn PreToolHook>>) -> Self {
        Self { hooks }
    }

    pub fn with_denied_commands(denied_patterns: Vec<String>) -> Self {
        Self {
            hooks: vec![Arc::new(CommandDenyHook::new(denied_patterns))],
        }
    }

    pub async fn evaluate(
        &self,
        tool_name: &str,
        arguments: &serde_json::Value,
        workspace_path: &Path,
    ) -> Result<PreToolDecision, SwellError> {
        for hook in &self.hooks {
            match hook.evaluate(tool_name, arguments, workspace_path).await? {
                PreToolDecision::Allow => {}
                decision @ PreToolDecision::Deny(_) => return Ok(decision),
            }
        }

        Ok(PreToolDecision::Allow)
    }
}

impl Default for PreToolHookManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn command_deny_hook_blocks_shell_pattern() {
        let hook = CommandDenyHook::new(vec!["rm -rf /".to_string()]);
        let decision = hook
            .evaluate(
                "shell",
                &serde_json::json!({ "command": "rm -rf /" }),
                tempdir().unwrap().path(),
            )
            .await
            .unwrap();

        assert!(matches!(decision, PreToolDecision::Deny(reason) if reason.contains("rm -rf /")));
    }

    #[tokio::test]
    async fn command_deny_hook_allows_unmatched_shell_command() {
        let hook = CommandDenyHook::new(vec!["rm -rf /".to_string()]);
        let decision = hook
            .evaluate(
                "shell",
                &serde_json::json!({ "command": "echo ok" }),
                tempdir().unwrap().path(),
            )
            .await
            .unwrap();

        assert_eq!(decision, PreToolDecision::Allow);
    }

    #[tokio::test]
    async fn command_deny_hook_blocks_git_force_pattern() {
        let hook = CommandDenyHook::new(vec!["reset --hard".to_string()]);
        let decision = hook
            .evaluate(
                "git",
                &serde_json::json!({ "operation": "reset", "args": ["--hard"] }),
                tempdir().unwrap().path(),
            )
            .await
            .unwrap();

        assert!(
            matches!(decision, PreToolDecision::Deny(reason) if reason.contains("reset --hard"))
        );
    }
}
