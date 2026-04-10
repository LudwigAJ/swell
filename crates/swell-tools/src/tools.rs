//! Built-in tools for SWELL.

use async_trait::async_trait;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use swell_core::traits::Tool;
use swell_core::traits::ToolBehavioralHints;
use swell_core::{PermissionTier, SwellError, ToolOutput, ToolRiskLevel};
use tempfile::NamedTempFile;
use tokio::fs as tokio_fs;
use tokio::time::{timeout, Duration};
use tracing::{info, warn};

/// Tool for reading files with workspace path validation
#[derive(Debug, Clone)]
pub struct ReadFileTool {
    max_size: usize,
    workspace_path: Option<PathBuf>,
}

impl ReadFileTool {
    pub fn new() -> Self {
        Self {
            max_size: 1_000_000, // 1MB default
            workspace_path: None,
        }
    }

    pub fn with_max_size(mut self, size: usize) -> Self {
        self.max_size = size;
        self
    }

    pub fn with_workspace_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.workspace_path = Some(path.into());
        self
    }

    /// Validate that the path is within the workspace boundaries
    fn validate_path(&self, path: &Path) -> Result<(), SwellError> {
        let canonical_path = path
            .canonicalize()
            .map_err(|e| SwellError::ToolExecutionFailed(format!("Cannot resolve path: {}", e)))?;

        if let Some(workspace) = &self.workspace_path {
            let canonical_workspace = workspace.canonicalize().map_err(|e| {
                SwellError::ToolExecutionFailed(format!("Cannot resolve workspace: {}", e))
            })?;

            if !canonical_path.starts_with(&canonical_workspace) {
                return Err(SwellError::ToolExecutionFailed(format!(
                    "Path '{}' is outside workspace boundaries",
                    path.display()
                )));
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> String {
        "Read the contents of a file".to_string()
    }
    fn risk_level(&self) -> ToolRiskLevel {
        ToolRiskLevel::Read
    }
    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::Auto
    }
    fn behavioral_hints(&self) -> ToolBehavioralHints {
        ToolBehavioralHints {
            read_only_hint: true,
            destructive_hint: false,
            idempotent_hint: true,
        }
    }

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
        struct Args {
            path: String,
        }

        let args: Args = serde_json::from_value(arguments)
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        let path = Path::new(&args.path);

        // Validate path is within workspace
        self.validate_path(path)?;

        if !path.exists() {
            return Err(SwellError::ToolExecutionFailed(format!(
                "File not found: {}",
                args.path
            )));
        }

        let metadata = tokio_fs::metadata(path)
            .await
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        if metadata.len() as usize > self.max_size {
            return Err(SwellError::ToolExecutionFailed(format!(
                "File too large: {} bytes (max: {})",
                metadata.len(),
                self.max_size
            )));
        }

        let content = tokio_fs::read_to_string(path)
            .await
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        info!(path = %args.path, "File read successfully");
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

/// Tool for writing files with atomic writes and rollback on failure
#[derive(Debug, Clone)]
pub struct WriteFileTool {
    max_size: usize,
}

impl WriteFileTool {
    pub fn new() -> Self {
        Self {
            max_size: 10_000_000,
        } // 10MB default
    }

    pub fn with_max_size(mut self, size: usize) -> Self {
        self.max_size = size;
        self
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }
    fn description(&self) -> String {
        "Write content to a file (creates or overwrites)".to_string()
    }
    fn risk_level(&self) -> ToolRiskLevel {
        ToolRiskLevel::Write
    }
    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::Ask
    }
    fn behavioral_hints(&self) -> ToolBehavioralHints {
        ToolBehavioralHints {
            read_only_hint: false,
            destructive_hint: false, // Uses atomic write, preserves original on failure
            idempotent_hint: true,
        }
    }

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
        struct Args {
            path: String,
            content: String,
        }

        let args: Args = serde_json::from_value(arguments)
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        if args.content.len() > self.max_size {
            return Err(SwellError::ToolExecutionFailed(format!(
                "Content too large: {} bytes (max: {})",
                args.content.len(),
                self.max_size
            )));
        }

        // Use atomic write with temporary file and rename for atomicity
        let path = Path::new(&args.path);

        // Create a temporary file in the same directory to ensure atomic rename works
        let temp_dir = path.parent().unwrap_or(Path::new("."));
        let temp_file = NamedTempFile::new_in(temp_dir).map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to create temp file: {}", e))
        })?;

        let temp_path = temp_file.into_temp_path();

        // Write content to temp file
        let sync_result = tokio_fs::write(&temp_path, &args.content).await;

        if let Err(e) = sync_result {
            // Temp file is dropped here, no rollback needed
            return Err(SwellError::ToolExecutionFailed(format!(
                "Failed to write content: {}",
                e
            )));
        }

        // Attempt to persist the temp file to the target path atomically
        // If this fails, the temp file is dropped without affecting the original
        let persist_result = temp_path.persist(path);

        match persist_result {
            Ok(_) => {
                info!(path = %args.path, "File written atomically");
                Ok(ToolOutput {
                    success: true,
                    result: format!(
                        "Wrote {} bytes to {} atomically",
                        args.content.len(),
                        args.path
                    ),
                    error: None,
                })
            }
            Err(e) => {
                // If persist failed, check if it's because the file already existed
                let error_msg = format!("Failed to persist file atomically: {}", e);
                warn!(path = %args.path, error = %error_msg, "Atomic write failed, original file preserved");
                Err(SwellError::ToolExecutionFailed(error_msg))
            }
        }
    }
}

impl Default for WriteFileTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Tool for executing shell commands with timeout support
#[derive(Debug, Clone)]
pub struct ShellTool {
    default_timeout_secs: u64,
}

impl ShellTool {
    pub fn new() -> Self {
        Self {
            default_timeout_secs: 60,
        }
    }

    pub fn with_default_timeout(mut self, secs: u64) -> Self {
        self.default_timeout_secs = secs;
        self
    }
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }
    fn description(&self) -> String {
        "Execute a shell command with timeout support".to_string()
    }
    fn risk_level(&self) -> ToolRiskLevel {
        ToolRiskLevel::Destructive
    }
    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::Deny
    }
    fn behavioral_hints(&self) -> ToolBehavioralHints {
        ToolBehavioralHints {
            read_only_hint: false,
            destructive_hint: true, // Can execute arbitrary commands including destructive ones
            idempotent_hint: false, // Depends on the command being executed
        }
    }

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
                    "description": "Timeout in seconds (default: 60)",
                    "default": 60
                },
                "working_dir": {
                    "type": "string",
                    "description": "Working directory for the command"
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
            working_dir: Option<String>,
        }

        let args: Args = serde_json::from_value(arguments)
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        let timeout_duration =
            Duration::from_secs(args.timeout_secs.unwrap_or(self.default_timeout_secs));

        let mut cmd = tokio::process::Command::new(&args.command);
        cmd.args(args.args.as_deref().unwrap_or(&[]));

        if let Some(ref dir) = args.working_dir {
            cmd.current_dir(dir);
        }

        // Execute with timeout
        let result = timeout(timeout_duration, cmd.output()).await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                info!(command = %args.command, "Shell command executed");
                Ok(ToolOutput {
                    success: output.status.success(),
                    result: stdout.to_string(),
                    error: if output.status.success() {
                        None
                    } else {
                        Some(stderr.to_string())
                    },
                })
            }
            Ok(Err(e)) => Err(SwellError::ToolExecutionFailed(format!(
                "Failed to execute command: {}",
                e
            ))),
            Err(_) => {
                // Timeout
                Err(SwellError::ToolExecutionFailed(format!(
                    "Command timed out after {} seconds",
                    timeout_duration.as_secs()
                )))
            }
        }
    }
}

impl Default for ShellTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Tool for git operations with structured commands
#[derive(Debug, Clone)]
pub struct GitTool {
    default_cwd: PathBuf,
}

impl GitTool {
    pub fn new() -> Self {
        Self {
            default_cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }

    pub fn with_default_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.default_cwd = cwd.into();
        self
    }

    async fn run_git(&self, args: &[&str], cwd: Option<&Path>) -> Result<ToolOutput, SwellError> {
        let output = tokio::process::Command::new("git")
            .args(args)
            .current_dir(cwd.unwrap_or(&self.default_cwd))
            .output()
            .await
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        Ok(ToolOutput {
            success: output.status.success(),
            result: stdout.to_string(),
            error: if output.status.success() {
                None
            } else {
                Some(stderr.to_string())
            },
        })
    }

    /// Execute git_status - returns structured status information
    async fn git_status(&self, cwd: &Path) -> Result<ToolOutput, SwellError> {
        let output = self
            .run_git(&["status", "--porcelain", "-b"], Some(cwd))
            .await?;

        if !output.success {
            return Ok(output);
        }

        // Parse git status output for structured response
        let lines: Vec<&str> = output.result.lines().collect();
        let mut status_info = serde_json::json!({
            "branch": "",
            "is_dirty": false,
            "staged": Vec::<String>::new(),
            "modified": Vec::<String>::new(),
            "untracked": Vec::<String>::new(),
        });

        if let Some(branch_line) = lines.first() {
            // Branch line format: ## branch-name
            if let Some(branch) = branch_line.strip_prefix("## ") {
                status_info["branch"] =
                    serde_json::json!(branch.split_whitespace().next().unwrap_or(""));
            }
        }

        let is_dirty =
            lines.len() > 1 || lines.first().map(|l| !l.starts_with("##")).unwrap_or(false);
        status_info["is_dirty"] = serde_json::json!(is_dirty);

        // Parse status lines (skip branch header)
        for line in lines.iter().skip(1) {
            if line.len() >= 3 {
                let staged = &line[0..1];
                let modified = &line[1..2];
                let untracked = &line[2..3];
                let path = &line[3..].trim();

                if staged != " " && staged != "?" {
                    status_info["staged"]
                        .as_array_mut()
                        .unwrap()
                        .push(serde_json::json!({
                            "status": staged,
                            "path": path
                        }));
                }
                if modified != " " {
                    status_info["modified"]
                        .as_array_mut()
                        .unwrap()
                        .push(serde_json::json!({
                            "status": modified,
                            "path": path
                        }));
                }
                if untracked == "?" {
                    status_info["untracked"]
                        .as_array_mut()
                        .unwrap()
                        .push(serde_json::json!(path));
                }
            }
        }

        Ok(ToolOutput {
            success: true,
            result: serde_json::to_string_pretty(&status_info).unwrap_or(output.result),
            error: None,
        })
    }

    /// Execute git_diff - returns structured diff information
    async fn git_diff(&self, args: &[String], cwd: &Path) -> Result<ToolOutput, SwellError> {
        let mut git_args = vec!["diff", "--stat"];
        git_args.extend(args.iter().map(|s| s.as_str()));

        self.run_git(&git_args, Some(cwd)).await
    }

    /// Execute git_log - returns structured commit history
    async fn git_log(&self, args: &[String], cwd: &Path) -> Result<ToolOutput, SwellError> {
        let mut git_args = vec!["log", "--oneline", "--decorate"];
        git_args.extend(args.iter().map(|s| s.as_str()));

        self.run_git(&git_args, Some(cwd)).await
    }

    /// Execute git_commit with metadata trailers
    async fn git_commit(
        &self,
        message: &str,
        metadata: Option<&str>,
        cwd: &Path,
    ) -> Result<ToolOutput, SwellError> {
        // Build commit message with metadata trailers
        let mut full_message = message.to_string();

        if let Some(meta) = metadata {
            full_message.push_str("\n\n");
            full_message.push_str(meta);
        }

        // Create commit with message
        self.run_git(&["commit", "-m", &full_message], Some(cwd))
            .await
    }

    /// Create a branch with naming convention: agent/<task-id>/<description>
    async fn create_branch(&self, branch_name: &str, cwd: &Path) -> Result<ToolOutput, SwellError> {
        self.run_git(&["checkout", "-b", branch_name], Some(cwd))
            .await
    }
}

#[derive(Debug, Deserialize)]
struct GitArgs {
    operation: String, // "status", "diff", "log", "commit", "branch"
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    metadata: Option<String>,
}

#[async_trait]
impl Tool for GitTool {
    fn name(&self) -> &str {
        "git"
    }
    fn description(&self) -> String {
        "Execute git operations (status, diff, log, commit, branch)".to_string()
    }
    fn risk_level(&self) -> ToolRiskLevel {
        ToolRiskLevel::Write
    }
    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::Ask
    }
    fn behavioral_hints(&self) -> ToolBehavioralHints {
        ToolBehavioralHints {
            read_only_hint: false,
            destructive_hint: false, // commit/branch operations are recoverable
            idempotent_hint: false,  // status/diff are idempotent, but commit/branch are not
        }
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "description": "Git operation: status, diff, log, commit, branch",
                    "enum": ["status", "diff", "log", "commit", "branch"]
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Additional arguments for the operation"
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory"
                },
                "message": {
                    "type": "string",
                    "description": "Commit message (for commit operation)"
                },
                "metadata": {
                    "type": "string",
                    "description": "Metadata trailers for commit (e.g., 'Generated-by: swell\\nTask-id: 123')"
                }
            },
            "required": ["operation"]
        })
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<ToolOutput, SwellError> {
        let args: GitArgs = serde_json::from_value(arguments)
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        let cwd = args
            .cwd
            .as_ref()
            .map(Path::new)
            .unwrap_or(&self.default_cwd);

        match args.operation.as_str() {
            "status" => self.git_status(cwd).await,
            "diff" => self.git_diff(&args.args, cwd).await,
            "log" => self.git_log(&args.args, cwd).await,
            "commit" => {
                let message = args.message.unwrap_or_else(|| "No message".to_string());
                self.git_commit(&message, args.metadata.as_deref(), cwd)
                    .await
            }
            "branch" => {
                if let Some(branch_name) = args.args.first() {
                    self.create_branch(branch_name, cwd).await
                } else {
                    Err(SwellError::ToolExecutionFailed(
                        "Branch name required".to_string(),
                    ))
                }
            }
            _ => Err(SwellError::ToolExecutionFailed(format!(
                "Unknown git operation: {}",
                args.operation
            ))),
        }
    }
}

impl Default for GitTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Tool for diff-based file modifications
#[derive(Debug, Clone)]
pub struct FileEditTool {
    max_diff_size: usize,
}

impl FileEditTool {
    pub fn new() -> Self {
        Self {
            max_diff_size: 1_000_000,
        } // 1MB default
    }

    pub fn with_max_diff_size(mut self, size: usize) -> Self {
        self.max_diff_size = size;
        self
    }

    /// Generate a unified diff between old and new content
    fn generate_diff(&self, path: &str, old_content: &str, new_content: &str) -> String {
        use std::io::Write;

        let mut diff_output = Vec::new();

        // Simple line-by-line diff generation
        let old_lines: Vec<&str> = old_content.lines().collect();
        let new_lines: Vec<&str> = new_content.lines().collect();

        writeln!(&mut diff_output, "--- a/{}", path).ok();
        writeln!(&mut diff_output, "+++ b/{}", path).ok();

        let max_lines = old_lines.len().max(new_lines.len());
        let mut line_num = 1;
        let mut in_hunk = false;

        for i in 0..max_lines {
            let old_line = old_lines.get(i);
            let new_line = new_lines.get(i);

            match (old_line, new_line) {
                (Some(old), Some(new)) if old == new => {
                    if in_hunk {
                        writeln!(&mut diff_output, " {}", new).ok();
                    }
                }
                (Some(old), Some(new)) => {
                    if !in_hunk {
                        writeln!(
                            &mut diff_output,
                            "@@ -{},{} +{},{} @@",
                            line_num,
                            old_lines.len().saturating_sub(i),
                            line_num,
                            new_lines.len().saturating_sub(i)
                        )
                        .ok();
                        in_hunk = true;
                    }
                    writeln!(&mut diff_output, "-{}", old).ok();
                    writeln!(&mut diff_output, "+{}", new).ok();
                }
                (Some(old), None) => {
                    if !in_hunk {
                        writeln!(
                            &mut diff_output,
                            "@@ -{},{} +{},{} @@",
                            line_num,
                            old_lines.len().saturating_sub(i),
                            line_num,
                            new_lines.len().saturating_sub(i)
                        )
                        .ok();
                        in_hunk = true;
                    }
                    writeln!(&mut diff_output, "-{}", old).ok();
                }
                (None, Some(new)) => {
                    if !in_hunk {
                        writeln!(
                            &mut diff_output,
                            "@@ -{},{} +{},{} @@",
                            line_num,
                            old_lines.len().saturating_sub(i),
                            line_num,
                            new_lines.len().saturating_sub(i)
                        )
                        .ok();
                        in_hunk = true;
                    }
                    writeln!(&mut diff_output, "+{}", new).ok();
                }
                _ => {}
            }
            line_num += 1;
        }

        String::from_utf8_lossy(&diff_output).to_string()
    }
}

#[derive(Debug, Deserialize)]
struct EditArgs {
    path: String,
    old_str: String,
    new_str: String,
    #[serde(default)]
    dry_run: bool,
}

#[async_trait]
impl Tool for FileEditTool {
    fn name(&self) -> &str {
        "edit_file"
    }
    fn description(&self) -> String {
        "Edit a file by replacing old_str with new_str. Shows diff before applying.".to_string()
    }
    fn risk_level(&self) -> ToolRiskLevel {
        ToolRiskLevel::Write
    }
    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::Ask
    }
    fn behavioral_hints(&self) -> ToolBehavioralHints {
        ToolBehavioralHints {
            read_only_hint: false,
            destructive_hint: false, // Atomic write preserves original on failure
            idempotent_hint: true,   // Same edit can be retried
        }
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file to edit" },
                "old_str": { "type": "string", "description": "String to replace" },
                "new_str": { "type": "string", "description": "Replacement string" },
                "dry_run": {
                    "type": "boolean",
                    "description": "If true, only show the diff without applying changes",
                    "default": false
                }
            },
            "required": ["path", "old_str", "new_str"]
        })
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<ToolOutput, SwellError> {
        let args: EditArgs = serde_json::from_value(arguments)
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        let path = Path::new(&args.path);

        // Read current content
        if !path.exists() {
            return Err(SwellError::ToolExecutionFailed(format!(
                "File not found: {}",
                args.path
            )));
        }

        let old_content = tokio_fs::read_to_string(path)
            .await
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        // Check if old_str exists in the file
        if !old_content.contains(&args.old_str) {
            return Err(SwellError::ToolExecutionFailed(
                "old_str not found in file. Please ensure the exact string exists.".to_string(),
            ));
        }

        // Generate diff
        let new_content = old_content.replace(&args.old_str, &args.new_str);
        let diff = self.generate_diff(&args.path, &old_content, &new_content);

        if args.dry_run {
            return Ok(ToolOutput {
                success: true,
                result: serde_json::json!({
                    "dry_run": true,
                    "diff": diff,
                    "would_change": old_content != new_content
                })
                .to_string(),
                error: None,
            });
        }

        // Apply the change using atomic write
        let temp_dir = path.parent().unwrap_or(Path::new("."));
        let temp_file = NamedTempFile::new_in(temp_dir).map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to create temp file: {}", e))
        })?;

        let temp_path = temp_file.into_temp_path();

        tokio_fs::write(&temp_path, &new_content)
            .await
            .map_err(|e| {
                SwellError::ToolExecutionFailed(format!("Failed to write changes: {}", e))
            })?;

        temp_path.persist(path).map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to save changes: {}", e))
        })?;

        info!(path = %args.path, "File edited successfully");
        Ok(ToolOutput {
            success: true,
            result: serde_json::json!({
                "dry_run": false,
                "diff": diff,
                "changed": true
            })
            .to_string(),
            error: None,
        })
    }
}

impl Default for FileEditTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Tool for searching files with grep, glob, and symbol search
#[derive(Debug, Clone)]
pub struct SearchTool {
    max_results: usize,
}

impl SearchTool {
    pub fn new() -> Self {
        Self { max_results: 1000 }
    }

    pub fn with_max_results(mut self, max: usize) -> Self {
        self.max_results = max;
        self
    }

    /// Execute grep search
    async fn grep(
        &self,
        pattern: &str,
        path: &str,
        case_insensitive: bool,
    ) -> Result<ToolOutput, SwellError> {
        let mut cmd = tokio::process::Command::new("grep");
        if case_insensitive {
            cmd.arg("-i");
        }
        cmd.args(["-n", "-r", "--"]);
        cmd.arg(pattern);
        cmd.arg(path);

        let output = cmd
            .output()
            .await
            .map_err(|e| SwellError::ToolExecutionFailed(format!("grep failed: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout);

        let results: Vec<&str> = stdout.lines().take(self.max_results).collect();

        Ok(ToolOutput {
            success: true,
            result: serde_json::json!({
                "matches": results,
                "count": results.len(),
                "pattern": pattern,
                "path": path
            })
            .to_string(),
            error: if !output.status.success() && results.is_empty() {
                Some("No matches found or grep error".to_string())
            } else {
                None
            },
        })
    }

    /// Execute glob search
    fn glob(&self, pattern: &str, base_path: &Path) -> Result<ToolOutput, SwellError> {
        use glob::glob as glob_match;

        let full_pattern = if pattern.starts_with('/') {
            pattern.to_string()
        } else {
            format!("{}/{}", base_path.display(), pattern)
        };

        let mut matches = Vec::new();

        if let Ok(globber) = glob_match(&full_pattern) {
            for entry in globber.flatten() {
                if matches.len() >= self.max_results {
                    break;
                }
                if entry.is_file() {
                    matches.push(entry.display().to_string());
                }
            }
        }

        Ok(ToolOutput {
            success: true,
            result: serde_json::json!({
                "matches": matches,
                "count": matches.len(),
                "pattern": pattern
            })
            .to_string(),
            error: None,
        })
    }

    /// Search for symbol definitions using grep
    async fn symbol_search(&self, symbol: &str, path: &str) -> Result<ToolOutput, SwellError> {
        // Common patterns for function/type definitions
        let patterns = [
            format!("^fn {} ", symbol),
            format!("^struct {} ", symbol),
            format!("^enum {} ", symbol),
            format!("^impl {} ", symbol),
            format!("^trait {} ", symbol),
            format!("^type {} ", symbol),
            format!("^const {} ", symbol),
            format!("macro_rules! {} ", symbol),
        ];

        let mut all_matches: Vec<String> = Vec::new();

        for pattern in &patterns {
            let mut cmd = tokio::process::Command::new("grep");
            cmd.args(["-n", "-r", "--"]);
            cmd.arg(pattern);
            cmd.arg(path);

            if let Ok(output) = cmd.output().await {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines().take(10) {
                    all_matches.push(line.to_string());
                }
            }
        }

        // Also do a simple grep for the symbol name
        let mut cmd = tokio::process::Command::new("grep");
        cmd.args(["-n", "-r", "-E", "--"]);
        cmd.arg(format!("\\b{}\\b", regex::escape(symbol)));
        cmd.arg(path);

        if let Ok(output) = cmd.output().await {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().take(self.max_results) {
                if !all_matches.contains(&line.to_string()) {
                    all_matches.push(line.to_string());
                }
            }
        }

        all_matches.truncate(self.max_results);

        Ok(ToolOutput {
            success: true,
            result: serde_json::json!({
                "matches": all_matches,
                "count": all_matches.len(),
                "symbol": symbol,
                "path": path
            })
            .to_string(),
            error: None,
        })
    }
}

#[derive(Debug, Deserialize)]
struct SearchArgs {
    operation: String, // "grep", "glob", "symbol_search"
    #[serde(default)]
    pattern: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    case_insensitive: Option<bool>,
}

#[async_trait]
impl Tool for SearchTool {
    fn name(&self) -> &str {
        "search"
    }
    fn description(&self) -> String {
        "Search tool for grep, glob, and symbol search operations".to_string()
    }
    fn risk_level(&self) -> ToolRiskLevel {
        ToolRiskLevel::Read
    }
    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::Auto
    }
    fn behavioral_hints(&self) -> ToolBehavioralHints {
        ToolBehavioralHints {
            read_only_hint: true, // All search operations are read-only
            destructive_hint: false,
            idempotent_hint: true,
        }
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "description": "Search operation: grep, glob, symbol_search",
                    "enum": ["grep", "glob", "symbol_search"]
                },
                "pattern": { "type": "string", "description": "Pattern to search for" },
                "path": {
                    "type": "string",
                    "description": "Path to search in (defaults to current directory)"
                },
                "case_insensitive": {
                    "type": "boolean",
                    "description": "Case insensitive search (for grep)",
                    "default": false
                }
            },
            "required": ["operation"]
        })
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<ToolOutput, SwellError> {
        let args: SearchArgs = serde_json::from_value(arguments)
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        match args.operation.as_str() {
            "grep" => {
                let pattern = args.pattern.unwrap_or_default();
                let path = args.path.as_deref().unwrap_or(".");
                let case_insensitive = args.case_insensitive.unwrap_or(false);
                self.grep(&pattern, path, case_insensitive).await
            }
            "glob" => {
                let pattern = args.pattern.unwrap_or_else(|| "*".to_string());
                let path = Path::new(args.path.as_deref().unwrap_or("."));
                self.glob(&pattern, path)
            }
            "symbol_search" => {
                let symbol = args.pattern.unwrap_or_default();
                let path = args.path.as_deref().unwrap_or(".");
                self.symbol_search(&symbol, path).await
            }
            _ => Err(SwellError::ToolExecutionFailed(format!(
                "Unknown search operation: {}",
                args.operation
            ))),
        }
    }
}

impl Default for SearchTool {
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
        let result = tool
            .execute(serde_json::json!({
                "path": file_path.to_str().unwrap()
            }))
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.result, "Hello, World!");
    }

    #[tokio::test]
    async fn test_read_file_tool_workspace_validation() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        tokio::fs::write(&file_path, "Hello!").await.unwrap();

        // Tool with workspace set to a different directory should fail
        let tool = ReadFileTool::new().with_workspace_path("/tmp/nonexistent");

        let result = tool
            .execute(serde_json::json!({
                "path": file_path.to_str().unwrap()
            }))
            .await;

        // Should fail because path is outside workspace
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_write_file_tool() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("output.txt");

        let tool = WriteFileTool::new();
        let result = tool
            .execute(serde_json::json!({
                "path": file_path.to_str().unwrap(),
                "content": "Test content"
            }))
            .await
            .unwrap();

        assert!(result.success);

        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(content, "Test content");
    }

    #[tokio::test]
    async fn test_shell_tool() {
        let tool = ShellTool::new();
        let result = tool
            .execute(serde_json::json!({
                "command": "echo",
                "args": ["Hello, Shell!"]
            }))
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.result.contains("Hello, Shell!"));
    }

    #[tokio::test]
    async fn test_shell_tool_with_timeout() {
        let tool = ShellTool::new().with_default_timeout(5);

        let result = tool
            .execute(serde_json::json!({
                "command": "sleep",
                "args": ["0.1"]
            }))
            .await
            .unwrap();

        assert!(result.success);
    }

    #[tokio::test]
    async fn test_shell_tool_timeout_expires() {
        let tool = ShellTool::new().with_default_timeout(1);

        let result = tool
            .execute(serde_json::json!({
                "command": "sleep",
                "args": ["5"]
            }))
            .await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("timed out"));
    }

    #[tokio::test]
    async fn test_git_tool_status() {
        let dir = tempdir().unwrap();

        // Initialize git repo
        tokio::process::Command::new("git")
            .args(["init"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();

        let tool = GitTool::new();
        let result = tool
            .execute(serde_json::json!({
                "operation": "status"
            }))
            .await
            .unwrap();

        assert!(result.success);
        // Should be valid JSON with branch info
        let parsed: serde_json::Value = serde_json::from_str(&result.result).unwrap();
        assert!(parsed["branch"].is_string());
    }

    #[tokio::test]
    async fn test_git_tool_commit_noop() {
        let dir = tempdir().unwrap();

        // Initialize git repo
        tokio::process::Command::new("git")
            .args(["init"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();

        let tool = GitTool::new();
        // Commit without any changes should fail
        let result = tool
            .execute(serde_json::json!({
                "operation": "commit",
                "message": "test commit",
                "cwd": dir.path().to_str().unwrap()
            }))
            .await;

        // Should fail because there's nothing to commit
        assert!(result.is_err() || !result.as_ref().unwrap().success);
    }

    #[tokio::test]
    async fn test_file_edit_tool_dry_run() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("edit_test.txt");
        tokio::fs::write(&file_path, "line1\nOLD_TEXT\nline3")
            .await
            .unwrap();

        let tool = FileEditTool::new();
        let result = tool
            .execute(serde_json::json!({
                "path": file_path.to_str().unwrap(),
                "old_str": "OLD_TEXT",
                "new_str": "NEW_TEXT",
                "dry_run": true
            }))
            .await
            .unwrap();

        assert!(result.success);
        let parsed: serde_json::Value = serde_json::from_str(&result.result).unwrap();
        assert!(parsed["dry_run"].as_bool().unwrap());
        assert!(parsed["diff"].as_str().unwrap().contains("OLD_TEXT"));
        assert!(parsed["diff"].as_str().unwrap().contains("NEW_TEXT"));

        // Verify original content unchanged
        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert!(content.contains("OLD_TEXT"));
    }

    #[tokio::test]
    async fn test_file_edit_tool_apply() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("edit_test.txt");
        tokio::fs::write(&file_path, "line1\nOLD_TEXT\nline3")
            .await
            .unwrap();

        let tool = FileEditTool::new();
        let result = tool
            .execute(serde_json::json!({
                "path": file_path.to_str().unwrap(),
                "old_str": "OLD_TEXT",
                "new_str": "NEW_TEXT",
                "dry_run": false
            }))
            .await
            .unwrap();

        assert!(result.success);

        // Verify content changed
        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert!(content.contains("NEW_TEXT"));
        assert!(!content.contains("OLD_TEXT"));
    }

    #[tokio::test]
    async fn test_file_edit_tool_not_found() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("nonexistent.txt");

        let tool = FileEditTool::new();
        let result = tool
            .execute(serde_json::json!({
                "path": file_path.to_str().unwrap(),
                "old_str": "something",
                "new_str": "other"
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_search_tool_grep() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("search_test.txt");
        tokio::fs::write(&file_path, "line1\nhello world\nline3\nHello again")
            .await
            .unwrap();

        let tool = SearchTool::new();
        let result = tool
            .execute(serde_json::json!({
                "operation": "grep",
                "pattern": "hello",
                "path": dir.path().to_str().unwrap()
            }))
            .await
            .unwrap();

        assert!(result.success);
        let parsed: serde_json::Value = serde_json::from_str(&result.result).unwrap();
        assert!(parsed["count"].as_i64().unwrap() >= 1);
    }

    #[tokio::test]
    async fn test_search_tool_glob() {
        let dir = tempdir().unwrap();
        let _file1 = tokio::fs::write(dir.path().join("test1.txt"), "content")
            .await
            .unwrap();
        let _file2 = tokio::fs::write(dir.path().join("test2.txt"), "content")
            .await
            .unwrap();

        let tool = SearchTool::new();
        let result = tool
            .execute(serde_json::json!({
                "operation": "glob",
                "pattern": "*.txt",
                "path": dir.path().to_str().unwrap()
            }))
            .await
            .unwrap();

        assert!(result.success);
        let parsed: serde_json::Value = serde_json::from_str(&result.result).unwrap();
        assert!(parsed["count"].as_i64().unwrap() >= 2);
    }

    #[tokio::test]
    async fn test_search_tool_symbol_search() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("symbol_test.rs");
        tokio::fs::write(file_path, "fn my_function() {}\nstruct MyStruct {}")
            .await
            .unwrap();

        let tool = SearchTool::new();
        let result = tool
            .execute(serde_json::json!({
                "operation": "symbol_search",
                "pattern": "my_function",
                "path": dir.path().to_str().unwrap()
            }))
            .await
            .unwrap();

        assert!(result.success);
    }

    // =============================================================================
    // Tool Behavioral Hint Annotation Tests
    // =============================================================================

    #[tokio::test]
    async fn test_read_file_annotations() {
        let tool = ReadFileTool::new();
        let hints = tool.behavioral_hints();
        assert!(hints.read_only_hint, "ReadFileTool should be read-only");
        assert!(
            !hints.destructive_hint,
            "ReadFileTool should not be destructive"
        );
        assert!(hints.idempotent_hint, "ReadFileTool should be idempotent");
    }

    #[tokio::test]
    async fn test_write_file_annotations() {
        let tool = WriteFileTool::new();
        let hints = tool.behavioral_hints();
        assert!(!hints.read_only_hint, "WriteFileTool should modify state");
        assert!(
            !hints.destructive_hint,
            "WriteFileTool uses atomic write and preserves original on failure"
        );
        assert!(hints.idempotent_hint, "WriteFileTool should be idempotent");
    }

    #[tokio::test]
    async fn test_shell_annotations() {
        let tool = ShellTool::new();
        let hints = tool.behavioral_hints();
        assert!(
            !hints.read_only_hint,
            "ShellTool can execute arbitrary commands"
        );
        assert!(hints.destructive_hint, "ShellTool can be destructive");
        assert!(
            !hints.idempotent_hint,
            "ShellTool idempotency depends on the command"
        );
    }

    #[tokio::test]
    async fn test_git_annotations() {
        let tool = GitTool::new();
        let hints = tool.behavioral_hints();
        assert!(!hints.read_only_hint, "GitTool modifies git state");
        assert!(
            !hints.destructive_hint,
            "GitTool operations are generally recoverable"
        );
        assert!(
            !hints.idempotent_hint,
            "GitTool commit/branch are not idempotent"
        );
    }

    #[tokio::test]
    async fn test_file_edit_annotations() {
        let tool = FileEditTool::new();
        let hints = tool.behavioral_hints();
        assert!(!hints.read_only_hint, "FileEditTool modifies files");
        assert!(
            !hints.destructive_hint,
            "FileEditTool preserves original on failure"
        );
        assert!(hints.idempotent_hint, "FileEditTool should be idempotent");
    }

    #[tokio::test]
    async fn test_search_annotations() {
        let tool = SearchTool::new();
        let hints = tool.behavioral_hints();
        assert!(hints.read_only_hint, "SearchTool should be read-only");
        assert!(
            !hints.destructive_hint,
            "SearchTool should not be destructive"
        );
        assert!(hints.idempotent_hint, "SearchTool should be idempotent");
    }
}
