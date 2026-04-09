//! Built-in tools for SWELL.

use swell_core::{ToolOutput, SwellError, ToolRiskLevel, PermissionTier};
use swell_core::traits::Tool;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::fs;
use tracing::{info, warn};

/// Tool for reading files
#[derive(Debug, Clone)]
pub struct ReadFileTool {
    max_size: usize,
}

impl ReadFileTool {
    pub fn new() -> Self {
        Self { max_size: 1_000_000 } // 1MB default
    }

    pub fn with_max_size(mut self, size: usize) -> Self {
        self.max_size = size;
        self
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str { "read_file" }
    fn description(&self) -> String { "Read the contents of a file".to_string() }
    fn risk_level(&self) -> ToolRiskLevel { ToolRiskLevel::Read }
    fn permission_tier(&self) -> PermissionTier { PermissionTier::Auto }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file to read" }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<ToolOutput, SwellError> {
        #[derive(Deserialize)]
        struct Args { path: String }

        let args: Args = serde_json::from_value(arguments)
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        let path = Path::new(&args.path);
        
        if !path.exists() {
            return Err(SwellError::ToolExecutionFailed(format!("File not found: {}", args.path)));
        }

        let metadata = fs::metadata(path).await
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        if metadata.len() as usize > self.max_size {
            return Err(SwellError::ToolExecutionFailed(format!(
                "File too large: {} bytes (max: {})", metadata.len(), self.max_size
            )));
        }

        let content = fs::read_to_string(path).await
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        Ok(ToolOutput {
            success: true,
            result: content,
            error: None,
        })
    }
}

impl Default for ReadFileTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Tool for writing files
#[derive(Debug, Clone)]
pub struct WriteFileTool {
    max_size: usize,
}

impl WriteFileTool {
    pub fn new() -> Self {
        Self { max_size: 10_000_000 } // 10MB default
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str { "write_file" }
    fn description(&self) -> String { "Write content to a file (creates or overwrites)".to_string() }
    fn risk_level(&self) -> ToolRiskLevel { ToolRiskLevel::Write }
    fn permission_tier(&self) -> PermissionTier { PermissionTier::Ask }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file to write" },
                "content": { "type": "string", "description": "Content to write" }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<ToolOutput, SwellError> {
        #[derive(Deserialize)]
        struct Args { path: String, content: String }

        let args: Args = serde_json::from_value(arguments)
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        if args.content.len() > self.max_size {
            return Err(SwellError::ToolExecutionFailed(format!(
                "Content too large: {} bytes (max: {})", args.content.len(), self.max_size
            )));
        }

        fs::write(&args.path, &args.content).await
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        info!(path = %args.path, "File written");
        Ok(ToolOutput {
            success: true,
            result: format!("Wrote {} bytes to {}", args.content.len(), args.path),
            error: None,
        })
    }
}

impl Default for WriteFileTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Tool for executing shell commands
#[derive(Debug, Clone)]
pub struct ShellTool;

impl ShellTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str { "shell" }
    fn description(&self) -> String { "Execute a shell command".to_string() }
    fn risk_level(&self) -> ToolRiskLevel { ToolRiskLevel::Destructive }
    fn permission_tier(&self) -> PermissionTier { PermissionTier::Deny }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Command to execute" },
                "args": { 
                    "type": "array", 
                    "items": { "type": "string" },
                    "description": "Command arguments" 
                },
                "timeout_secs": { 
                    "type": "integer", 
                    "description": "Timeout in seconds",
                    "default": 60
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<ToolOutput, SwellError> {
        #[derive(Deserialize)]
        struct Args { 
            command: String, 
            args: Option<Vec<String>>,
            timeout_secs: Option<u64>,
        }

        let args: Args = serde_json::from_value(arguments)
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        let output = tokio::process::Command::new(&args.command)
            .args(args.args.as_deref().unwrap_or(&[]))
            .output()
            .await
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        Ok(ToolOutput {
            success: output.status.success(),
            result: stdout.to_string(),
            error: if output.status.success() { None } else { Some(stderr.to_string()) },
        })
    }
}

impl Default for ShellTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Tool for git operations
#[derive(Debug, Clone)]
pub struct GitTool;

impl GitTool {
    pub fn new() -> Self {
        Self
    }

    async fn run_git(&self, args: &[&str], cwd: Option<&Path>) -> Result<ToolOutput, SwellError> {
        let output = tokio::process::Command::new("git")
            .args(args)
            .current_dir(cwd.unwrap_or_else(|| Path::new(".")))
            .output()
            .await
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        Ok(ToolOutput {
            success: output.status.success(),
            result: stdout.to_string(),
            error: if output.status.success() { None } else { Some(stderr.to_string()) },
        })
    }
}

#[async_trait]
impl Tool for GitTool {
    fn name(&self) -> &str { "git" }
    fn description(&self) -> String { "Execute git commands".to_string() }
    fn risk_level(&self) -> ToolRiskLevel { ToolRiskLevel::Write }
    fn permission_tier(&self) -> PermissionTier { PermissionTier::Ask }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "args": { 
                    "type": "array", 
                    "items": { "type": "string" },
                    "description": "Git arguments (e.g., [\"status\"], [\"commit\", \"-m\", \"message\"])" 
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory"
                }
            },
            "required": ["args"]
        })
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<ToolOutput, SwellError> {
        #[derive(Deserialize)]
        struct Args { args: Vec<String>, cwd: Option<String> }

        let args: Args = serde_json::from_value(arguments)
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        let cwd = args.cwd.as_ref().map(Path::new);
        let git_args: Vec<&str> = args.args.iter().map(|s| s.as_str()).collect();

        self.run_git(&git_args, cwd).await
    }
}

impl Default for GitTool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_read_file_tool() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        tokio::fs::write(&file_path, "Hello, World!").await.unwrap();

        let tool = ReadFileTool::new();
        let result = tool.execute(serde_json::json!({
            "path": file_path.to_str().unwrap()
        })).await.unwrap();

        assert!(result.success);
        assert_eq!(result.result, "Hello, World!");
    }

    #[tokio::test]
    async fn test_write_file_tool() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("output.txt");

        let tool = WriteFileTool::new();
        let result = tool.execute(serde_json::json!({
            "path": file_path.to_str().unwrap(),
            "content": "Test content"
        })).await.unwrap();

        assert!(result.success);

        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(content, "Test content");
    }

    #[tokio::test]
    async fn test_shell_tool() {
        let tool = ShellTool;
        let result = tool.execute(serde_json::json!({
            "command": "echo",
            "args": ["Hello, Shell!"]
        })).await.unwrap();

        assert!(result.success);
        assert!(result.result.contains("Hello, Shell!"));
    }

    #[tokio::test]
    async fn test_git_tool() {
        let dir = tempdir().unwrap();
        
        // Initialize git repo
        tokio::process::Command::new("git")
            .args(["init"])
            .current_dir(&dir)
            .output()
            .await.unwrap();

        let tool = GitTool;
        let result = tool.execute(serde_json::json!({
            "args": ["status"],
            "cwd": dir.path().to_str().unwrap()
        })).await.unwrap();

        assert!(result.success);
    }
}
