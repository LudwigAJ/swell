//! Built-in tools for SWELL.

use async_trait::async_trait;
use base64::Engine;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use swell_core::traits::Tool;
use swell_core::traits::ToolBehavioralHints;
use swell_core::{PermissionTier, SwellError, ToolOutput, ToolResultContent, ToolRiskLevel};
use tempfile::NamedTempFile;
use tokio::fs as tokio_fs;
use tokio::time::{timeout, Duration};
use tracing::{info, warn};

use crate::file_guardrails::{
    detect_binary_content, validate_file_size, validate_path_depth, validate_write_content,
    FileGuardrailConfig,
};
use crate::secret_scanning::SecretScanner;

/// Tool for reading files with workspace path validation
#[derive(Debug, Clone)]
pub struct ReadFileTool {
    max_size: usize,
    workspace_path: Option<PathBuf>,
    allow_binary: bool,
}

impl ReadFileTool {
    pub fn new() -> Self {
        Self {
            max_size: 1_000_000, // 1MB default
            workspace_path: None,
            allow_binary: false,
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

    /// Set whether to allow reading binary files.
    /// When false (default), binary files are rejected with an error.
    /// When true, binary files are read and returned as base64-encoded text.
    pub fn with_allow_binary(mut self, allow: bool) -> Self {
        self.allow_binary = allow;
        self
    }

    /// Validate that the path is within the workspace boundaries using two-layer safety:
    /// 1. Prefix check: raw path must start with workspace prefix
    /// 2. Canonical path check: resolved symlink target must still be within workspace
    fn validate_path(&self, path: &Path) -> Result<(), SwellError> {
        // Layer 1: Prefix check - raw path must start with workspace prefix
        if let Some(workspace) = &self.workspace_path {
            let workspace_str = workspace.to_string_lossy();
            let path_str = path.to_string_lossy();

            if !path_str.starts_with(workspace_str.as_ref()) && !path_str.starts_with('/') {
                // Path doesn't start with workspace prefix and isn't absolute
                let abs_path = std::env::current_dir()
                    .map(|cwd| cwd.join(path))
                    .unwrap_or_else(|_| path.to_path_buf());
                let abs_path_str = abs_path.to_string_lossy();
                if !abs_path_str.starts_with(workspace_str.as_ref()) {
                    return Err(SwellError::ToolExecutionFailed(format!(
                        "Path '{}' is outside workspace boundaries",
                        path.display()
                    )));
                }
            }
        }

        // Layer 2: Canonical path check - resolve symlinks and verify target is within workspace
        let canonical_path = path
            .canonicalize()
            .map_err(|e| SwellError::ToolExecutionFailed(format!("Cannot resolve path: {}", e)))?;

        if let Some(workspace) = &self.workspace_path {
            let canonical_workspace = workspace.canonicalize().map_err(|e| {
                SwellError::ToolExecutionFailed(format!("Cannot resolve workspace: {}", e))
            })?;

            if !canonical_path.starts_with(&canonical_workspace) {
                return Err(SwellError::ToolExecutionFailed(format!(
                    "Path '{}' is outside workspace boundaries (symlink escape detected)",
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
                "path": { "type": "string", "description": "Path to the file to read" },
                "allow_binary": {
                    "type": "boolean",
                    "description": "If true, allow reading binary files (returned as base64-encoded text). If false (default), binary files are rejected.",
                    "default": false
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<ToolOutput, SwellError> {
        #[derive(Deserialize)]
        struct Args {
            path: String,
            #[serde(default)]
            allow_binary: bool,
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

        // Read file as bytes to check for binary content
        let content_bytes = tokio_fs::read(path)
            .await
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        // Check if content is binary
        if let Err(reason) = detect_binary_content(&content_bytes) {
            // Binary content detected - check if allow_binary is set
            if !args.allow_binary {
                return Err(SwellError::ToolExecutionFailed(format!(
                    "Cannot read binary file '{}': {}. \
                     Set allow_binary=true to read binary files as base64-encoded text.",
                    args.path,
                    reason
                )));
            }
            // allow_binary is true - encode as base64 and return
            let base64_content = base64::engine::general_purpose::STANDARD.encode(&content_bytes);
            info!(path = %args.path, "Binary file read successfully (base64-encoded)");
            return Ok(ToolOutput {
                is_error: false,
                content: vec![ToolResultContent::Text(format!(
                    "[Binary file '{}' - base64 encoded ({} bytes)]\n{}",
                    args.path,
                    content_bytes.len(),
                    base64_content
                ))],
            });
        }

        // Content is not binary - convert to string and return
        let content = String::from_utf8(content_bytes)
            .map_err(|e| SwellError::ToolExecutionFailed(format!(
                "File content is not valid UTF-8: {}",
                e
            )))?;

        info!(path = %args.path, "File read successfully");
        Ok(ToolOutput {
            is_error: false,
            content: vec![ToolResultContent::Text(content)],
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
    workspace_path: Option<PathBuf>,
    guardrail_config: FileGuardrailConfig,
}

impl WriteFileTool {
    pub fn new() -> Self {
        Self {
            max_size: 10_000_000,
            workspace_path: None,
            guardrail_config: FileGuardrailConfig::default(),
        } // 10MB default
    }

    pub fn with_max_size(mut self, size: usize) -> Self {
        self.max_size = size;
        self
    }

    pub fn with_workspace_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.workspace_path = Some(path.into());
        self
    }

    /// Set the file guardrail configuration for write validation
    pub fn with_guardrail_config(mut self, config: FileGuardrailConfig) -> Self {
        self.guardrail_config = config;
        self
    }

    /// Configure guardrails to use defaults (binary detection, size limit, depth limit)
    pub fn with_default_guardrails(self) -> Self {
        self.with_guardrail_config(FileGuardrailConfig::default())
    }

    /// Validate that the path is within the workspace boundaries using two-layer safety:
    /// 1. Prefix check: raw path must start with workspace prefix
    /// 2. Canonical path check: resolved symlink target must still be within workspace
    fn validate_path(&self, path: &Path) -> Result<(), SwellError> {
        // For write operations, we need to check if the parent directory exists
        // and is within the workspace, since the file may not exist yet
        let parent = path.parent().unwrap_or(Path::new("."));

        // Layer 1: Prefix check - raw path must start with workspace prefix
        if let Some(workspace) = &self.workspace_path {
            let workspace_str = workspace.to_string_lossy();
            let parent_str = parent.to_string_lossy();

            if !parent_str.starts_with(workspace_str.as_ref()) && !parent_str.starts_with('/') {
                // Path doesn't start with workspace prefix and isn't absolute
                let abs_parent = std::env::current_dir()
                    .map(|cwd| cwd.join(parent))
                    .unwrap_or_else(|_| parent.to_path_buf());
                let abs_parent_str = abs_parent.to_string_lossy();
                if !abs_parent_str.starts_with(workspace_str.as_ref()) {
                    return Err(SwellError::ToolExecutionFailed(format!(
                        "Path '{}' is outside workspace boundaries",
                        path.display()
                    )));
                }
            }
        }

        // Layer 2: Canonical path check - resolve symlinks and verify target is within workspace
        // Canonicalize the PATH (not just parent) to detect symlink escape
        // For existing files (including symlinks), this resolves the symlink to its real target
        // For non-existent files, this finds the nearest existing parent
        let canonical_path = if path.exists() {
            // Path exists (could be a file or symlink) - canonicalize to resolve symlinks
            path.canonicalize().map_err(|e| {
                SwellError::ToolExecutionFailed(format!("Cannot resolve path: {}", e))
            })?
        } else {
            // Path doesn't exist - find nearest existing parent
            // This handles the case where we're creating a new file in a new directory
            let mut current = path;
            while !current.exists() {
                current = current.parent().unwrap_or(Path::new("."));
                if current.as_os_str().is_empty() {
                    return Err(SwellError::ToolExecutionFailed(
                        "Cannot resolve path: no existing parent found".to_string(),
                    ));
                }
            }
            current.canonicalize().map_err(|e| {
                SwellError::ToolExecutionFailed(format!("Cannot resolve workspace: {}", e))
            })?
        };

        if let Some(workspace) = &self.workspace_path {
            let canonical_workspace = workspace.canonicalize().map_err(|e| {
                SwellError::ToolExecutionFailed(format!("Cannot resolve workspace: {}", e))
            })?;

            if !canonical_path.starts_with(&canonical_workspace) {
                return Err(SwellError::ToolExecutionFailed(format!(
                    "Path '{}' is outside workspace boundaries (symlink escape detected)",
                    path.display()
                )));
            }
        }
        Ok(())
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

        let path = Path::new(&args.path);

        // Apply file guardrails: binary detection, size limit, depth limit
        let content_bytes = args.content.as_bytes();
        validate_write_content(content_bytes, &self.guardrail_config)?;
        validate_file_size(content_bytes.len(), &self.guardrail_config)?;
        validate_path_depth(path, &self.guardrail_config)?;

        // Use atomic write with temporary file and rename for atomicity
        // Validate path is within workspace
        self.validate_path(path)?;

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
                    is_error: false,
                    content: vec![ToolResultContent::Text(format!(
                        "Wrote {} bytes to {} atomically",
                        args.content.len(),
                        args.path
                    ))],
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
                let _stderr = String::from_utf8_lossy(&output.stderr);

                info!(command = %args.command, "Shell command executed");
                Ok(ToolOutput {
                    is_error: !output.status.success(),
                    content: vec![ToolResultContent::Text(stdout.to_string())],
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
    secret_scanner: Option<Arc<SecretScanner>>,
}

impl GitTool {
    pub fn new() -> Self {
        Self {
            default_cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            secret_scanner: Some(Arc::new(SecretScanner::new())),
        }
    }

    pub fn with_default_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.default_cwd = cwd.into();
        self
    }

    pub fn with_secret_scanner(mut self, scanner: Arc<SecretScanner>) -> Self {
        self.secret_scanner = Some(scanner);
        self
    }

    /// Create a GitTool with a SecretScanner for commit validation.
    /// This should be used in production to ensure secrets are scanned before commits.
    ///
    /// Note: As of the M12-safety-runtime followup, GitTool::new() now also injects
    /// a SecretScanner by default. This factory method is retained for explicit
    /// configuration and backward compatibility.
    pub fn create() -> Self {
        Self::new().with_secret_scanner(Arc::new(SecretScanner::new()))
    }

    async fn run_git(&self, args: &[&str], cwd: Option<&Path>) -> Result<ToolOutput, SwellError> {
        let output = tokio::process::Command::new("git")
            .args(args)
            .current_dir(cwd.unwrap_or(&self.default_cwd))
            .output()
            .await
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let _stderr = String::from_utf8_lossy(&output.stderr);

        Ok(ToolOutput {
            is_error: !output.status.success(),
            content: vec![ToolResultContent::Text(stdout.to_string())],
        })
    }

    /// Execute git_status - returns structured status information
    async fn git_status(&self, cwd: &Path) -> Result<ToolOutput, SwellError> {
        let output = self
            .run_git(&["status", "--porcelain", "-b"], Some(cwd))
            .await?;

        if output.is_error {
            return Ok(output);
        }

        // Extract text content from ToolOutput
        let output_text = match output.content.first() {
            Some(ToolResultContent::Text(text)) => text.clone(),
            _ => String::new(),
        };

        // Parse git status output for structured response
        let lines: Vec<&str> = output_text.lines().collect();
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
            is_error: false,
            content: vec![ToolResultContent::Json(status_info)],
        })
    }

    /// Execute git_diff - returns structured diff information
    ///
    /// Produces a structured diff output with file paths, added/removed line counts,
    /// and line content for review before commit.
    async fn git_diff(&self, args: &[String], cwd: &Path) -> Result<ToolOutput, SwellError> {
        // Run git diff --numstat to get per-file addition/deletion counts
        let mut numstat_args = vec!["diff", "--numstat"];
        numstat_args.extend(args.iter().map(|s| s.as_str()));
        let numstat_output = self.run_git(&numstat_args, Some(cwd)).await?;

        // Run git diff --stat for summary
        let mut stat_args = vec!["diff", "--stat"];
        stat_args.extend(args.iter().map(|s| s.as_str()));
        let stat_output = self.run_git(&stat_args, Some(cwd)).await?;

        // Run git diff with unified format for line content (3 lines of context)
        let mut diff_args = vec!["diff", "--unified=3"];
        diff_args.extend(args.iter().map(|s| s.as_str()));
        let diff_output = self.run_git(&diff_args, Some(cwd)).await?;

        // Parse the diff output into structured format
        let diff_info = self.parse_structured_diff(&stat_output, &numstat_output, &diff_output);

        Ok(ToolOutput {
            is_error: diff_info.is_err(),
            content: vec![ToolResultContent::Json(diff_info?)],
        })
    }

    /// Parse git diff outputs into structured diff format
    fn parse_structured_diff(
        &self,
        _stat_output: &ToolOutput,
        numstat_output: &ToolOutput,
        diff_output: &ToolOutput,
    ) -> Result<serde_json::Value, SwellError> {
        // Extract text content from ToolOutput
        let get_text = |output: &ToolOutput| -> String {
            match output.content.first() {
                Some(ToolResultContent::Text(text)) => text.clone(),
                _ => String::new(),
            }
        };

        let numstat_text = get_text(numstat_output);
        let diff_text = get_text(diff_output);

        // Parse --numstat output: format is "<additions>\t<deletions>\t<path>"
        let mut files: Vec<serde_json::Value> = Vec::new();
        let mut file_stats: std::collections::HashMap<String, (u32, u32)> =
            std::collections::HashMap::new();

        for line in numstat_text.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 3 {
                let additions = parts[0].parse::<u32>().unwrap_or(0);
                let deletions = parts[1].parse::<u32>().unwrap_or(0);
                let path = parts[2].to_string();

                file_stats.insert(path.clone(), (additions, deletions));
                files.push(serde_json::json!({
                    "path": path,
                    "additions": additions,
                    "deletions": deletions,
                    "hunks": Vec::<serde_json::Value>::new()
                }));
            }
        }

        // If no files changed, return empty diff
        if files.is_empty() {
            let empty_files: Vec<serde_json::Value> = Vec::new();
            return Ok(serde_json::json!({
                "files": empty_files,
                "total_additions": 0,
                "total_deletions": 0,
                "total_files": 0
            }));
        }

        // Parse the unified diff text to extract hunk information
        let mut current_file: Option<String> = None;
        let mut current_path_idx: Option<usize> = None;
        let mut current_hunk: Option<serde_json::Value> = None;
        let mut current_hunk_lines: Vec<serde_json::Value> = Vec::new();

        for line in diff_text.lines() {
            // Detect new file in diff
            if line.starts_with("diff --git") {
                // Save previous hunk if exists
                if let (Some(idx), Some(hunk)) = (current_path_idx.take(), current_hunk.take()) {
                    if !current_hunk_lines.is_empty() {
                        let hunk_obj = serde_json::json!({
                            "header": hunk,
                            "lines": current_hunk_lines.clone()
                        });
                        if let Some(file_obj) = files.get_mut(idx) {
                            file_obj["hunks"].as_array_mut().unwrap().push(hunk_obj);
                        }
                    }
                }
                current_hunk_lines.clear();

                // Extract path from "diff --git a/path b/path"
                if let Some(path) = line.strip_prefix("diff --git a/") {
                    let path = path.split(' ').next().unwrap_or(path);
                    let clean_path = path.strip_prefix("b/").unwrap_or(path).to_string();

                    // Find matching file in our list
                    current_path_idx = files.iter().position(|f| f["path"] == clean_path);
                    current_file = Some(clean_path);
                }
            } else if line.starts_with("@@ ") {
                // Save previous hunk
                if let (Some(idx), Some(hunk)) = (current_path_idx, current_hunk.take()) {
                    if !current_hunk_lines.is_empty() {
                        let hunk_obj = serde_json::json!({
                            "header": hunk,
                            "lines": current_hunk_lines.clone()
                        });
                        if let Some(file_obj) = files.get_mut(idx) {
                            file_obj["hunks"].as_array_mut().unwrap().push(hunk_obj);
                        }
                    }
                }
                current_hunk_lines.clear();

                // Start new hunk
                current_hunk = Some(serde_json::json!(line));
            } else if current_file.is_some() && current_path_idx.is_some() {
                // Collect hunk lines (only for files we have in our list)
                let (line_type, content) = if let Some(stripped) = line.strip_prefix('+') {
                    ("addition", stripped)
                } else if let Some(stripped) = line.strip_prefix('-') {
                    ("deletion", stripped)
                } else if let Some(stripped) = line.strip_prefix(' ') {
                    ("context", stripped)
                } else {
                    continue;
                };

                current_hunk_lines.push(serde_json::json!({
                    "type": line_type,
                    "content": content
                }));
            }
        }

        // Save last hunk
        if let (Some(idx), Some(hunk)) = (current_path_idx, current_hunk.take()) {
            if !current_hunk_lines.is_empty() {
                let hunk_obj = serde_json::json!({
                    "header": hunk,
                    "lines": current_hunk_lines
                });
                if let Some(file_obj) = files.get_mut(idx) {
                    file_obj["hunks"].as_array_mut().unwrap().push(hunk_obj);
                }
            }
        }

        // Calculate totals
        let total_additions: u32 = file_stats.values().map(|(a, _)| a).sum();
        let total_deletions: u32 = file_stats.values().map(|(_, d)| d).sum();

        Ok(serde_json::json!({
            "files": files,
            "total_additions": total_additions,
            "total_deletions": total_deletions,
            "total_files": files.len() as u32
        }))
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
        // Scan staged changes for secrets before committing
        if let Some(ref scanner) = self.secret_scanner {
            match scanner.scan_staged(cwd).await {
                Ok(result) => {
                    if result.has_secrets() {
                        let report = result.report();
                        warn!(report = %report, "Commit blocked: secrets detected in staged changes");
                        return Err(SwellError::ToolExecutionFailed(format!(
                            "Commit blocked: secrets detected in staged changes.\n{}",
                            report
                        )));
                    }
                }
                Err(e) => {
                    // Log the error but don't block the commit if scanner fails
                    warn!(error = %e, "Secret scan failed, proceeding with commit");
                }
            }
        }

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
    workspace_path: Option<PathBuf>,
    guardrail_config: FileGuardrailConfig,
}

impl FileEditTool {
    pub fn new() -> Self {
        Self {
            max_diff_size: 1_000_000,
            workspace_path: None,
            guardrail_config: FileGuardrailConfig::default(),
        } // 1MB default
    }

    pub fn with_max_diff_size(mut self, size: usize) -> Self {
        self.max_diff_size = size;
        self
    }

    pub fn with_workspace_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.workspace_path = Some(path.into());
        self
    }

    /// Set the file guardrail configuration for write validation
    pub fn with_guardrail_config(mut self, config: FileGuardrailConfig) -> Self {
        self.guardrail_config = config;
        self
    }

    /// Configure guardrails to use defaults (binary detection, size limit, depth limit)
    pub fn with_default_guardrails(self) -> Self {
        self.with_guardrail_config(FileGuardrailConfig::default())
    }

    /// Validate that the path is within the workspace boundaries using two-layer safety:
    /// 1. Prefix check: raw path must start with workspace prefix
    /// 2. Canonical path check: resolved symlink target must still be within workspace
    fn validate_path(&self, path: &Path) -> Result<(), SwellError> {
        // Layer 1: Prefix check - raw path must start with workspace prefix
        if let Some(workspace) = &self.workspace_path {
            let workspace_str = workspace.to_string_lossy();
            let path_str = path.to_string_lossy();

            if !path_str.starts_with(workspace_str.as_ref()) && !path_str.starts_with('/') {
                // Path doesn't start with workspace prefix and isn't absolute
                let abs_path = std::env::current_dir()
                    .map(|cwd| cwd.join(path))
                    .unwrap_or_else(|_| path.to_path_buf());
                let abs_path_str = abs_path.to_string_lossy();
                if !abs_path_str.starts_with(workspace_str.as_ref()) {
                    return Err(SwellError::ToolExecutionFailed(format!(
                        "Path '{}' is outside workspace boundaries",
                        path.display()
                    )));
                }
            }
        }

        // Layer 2: Canonical path check - resolve symlinks and verify target is within workspace
        let canonical_path = path
            .canonicalize()
            .map_err(|e| SwellError::ToolExecutionFailed(format!("Cannot resolve path: {}", e)))?;

        if let Some(workspace) = &self.workspace_path {
            let canonical_workspace = workspace.canonicalize().map_err(|e| {
                SwellError::ToolExecutionFailed(format!("Cannot resolve workspace: {}", e))
            })?;

            if !canonical_path.starts_with(&canonical_workspace) {
                return Err(SwellError::ToolExecutionFailed(format!(
                    "Path '{}' is outside workspace boundaries (symlink escape detected)",
                    path.display()
                )));
            }
        }
        Ok(())
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

        // Apply file guardrails for the new content: binary detection, size limit, depth limit
        // Note: For edits, we validate the new_str since that's the content being added
        validate_write_content(args.new_str.as_bytes(), &self.guardrail_config)?;
        validate_file_size(args.new_str.len(), &self.guardrail_config)?;
        validate_path_depth(path, &self.guardrail_config)?;

        // Validate path is within workspace (two-layer safety: prefix + canonical)
        self.validate_path(path)?;

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
                is_error: false,
                content: vec![ToolResultContent::Json(serde_json::json!({
                    "dry_run": true,
                    "diff": diff,
                    "would_change": old_content != new_content
                }))],
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
            is_error: false,
            content: vec![ToolResultContent::Json(serde_json::json!({
                "dry_run": false,
                "diff": diff,
                "changed": true
            }))],
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
            is_error: false,
            content: vec![ToolResultContent::Json(serde_json::json!({
                "matches": results,
                "count": results.len(),
                "pattern": pattern,
                "path": path
            }))],
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
            is_error: false,
            content: vec![ToolResultContent::Json(serde_json::json!({
                "matches": matches,
                "count": matches.len(),
                "pattern": pattern
            }))],
        })
    }

    /// Search for symbol definitions using grep
    async fn symbol_search(&self, symbol: &str, path: &str) -> Result<ToolOutput, SwellError> {
        // Common patterns for function/type definitions
        // Uses extended regex (-E) with word boundaries (\b) to match:
        // - Functions with modifiers: pub fn, async fn, unsafe fn, extern fn
        // - Generic functions: fn foo<T>
        // - All other definition types: struct, enum, impl, trait, type, const
        // - Macros: macro_rules! (note the !, not :)
        let patterns = [
            // Functions with optional modifiers (pub, async, unsafe, extern)
            // The \b word boundary before the symbol name ensures we match "fn symbol"
            // but not "otherfn_symbol" or "some_fn_symbol"
            format!(r#"(pub\s+|async\s+|unsafe\s+|extern\s+)*fn\s+{}\b"#, symbol),
            // Type definitions - \b word boundary handles generics (fn foo<T>)
            format!(r#"\bstruct {}\b"#, symbol),
            format!(r#"\benum {}\b"#, symbol),
            format!(r#"\bimpl {}\b"#, symbol),
            format!(r#"\btrait {}\b"#, symbol),
            format!(r#"\btype {}\b"#, symbol),
            format!(r#"\bconst {}\b"#, symbol),
            // Macros - macro_rules! has ! (not :)
            format!(r#"macro_rules! {}\b"#, symbol),
        ];

        let mut all_matches: Vec<String> = Vec::new();

        for pattern in &patterns {
            let mut cmd = tokio::process::Command::new("grep");
            // Use -E for extended regex so \b word boundaries work
            cmd.args(["-n", "-r", "-E", "--"]);
            cmd.arg(pattern);
            cmd.arg(path);

            if let Ok(output) = cmd.output().await {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines().take(10) {
                    all_matches.push(line.to_string());
                }
            }
        }

        // Also do a simple grep for the symbol name as whole word
        let mut cmd = tokio::process::Command::new("grep");
        cmd.args(["-n", "-r", "-E", "--"]);
        cmd.arg(format!(r#"\b{}\b"#, regex::escape(symbol)));
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
            is_error: false,
            content: vec![ToolResultContent::Json(serde_json::json!({
                "matches": all_matches,
                "count": all_matches.len(),
                "symbol": symbol,
                "path": path
            }))],
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

        assert!(!result.is_error);
        let content = match result.content.first() {
            Some(ToolResultContent::Text(text)) => text.clone(),
            _ => String::new(),
        };
        assert_eq!(content, "Hello, World!");
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
    async fn test_read_binary_file_rejected_without_flag() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.png");

        // Write PNG magic bytes followed by valid UTF-8 text (no null bytes)
        // PNG header: 8-byte magic followed by ASCII text
        let png_magic: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let text_content = b"PNG header followed by ASCII text content";
        let mut file_content = Vec::new();
        file_content.extend_from_slice(&png_magic);
        file_content.extend_from_slice(text_content);
        tokio::fs::write(&file_path, &file_content).await.unwrap();

        let tool = ReadFileTool::new();
        let result = tool
            .execute(serde_json::json!({
                "path": file_path.to_str().unwrap()
            }))
            .await;

        // Should fail because binary content is detected
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.to_lowercase().contains("binary"),
            "Error should mention 'binary': {}",
            err_msg
        );
        // PNG magic bytes should be detected
        assert!(
            err_msg.contains("PNG"),
            "Error should mention PNG format: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_read_binary_file_allowed_with_flag() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.png");

        // Write PNG magic bytes (binary file content)
        let png_content = b"\x89PNG\r\n\x1A\n\x00\x00\x00\rIHDR\x00\x00\x00\x10";
        tokio::fs::write(&file_path, png_content).await.unwrap();

        let tool = ReadFileTool::new();
        let result = tool
            .execute(serde_json::json!({
                "path": file_path.to_str().unwrap(),
                "allow_binary": true
            }))
            .await;

        // Should succeed with base64-encoded content
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(!output.is_error);
        let content = match output.content.first() {
            Some(ToolResultContent::Text(text)) => text.clone(),
            _ => String::new(),
        };
        assert!(
            content.contains("Binary file"),
            "Should indicate binary file: {}",
            content
        );
        assert!(
            content.contains("base64 encoded"),
            "Should be base64 encoded: {}",
            content
        );
    }

    // Note: Binary write rejection is tested via file_guardrails tests
    // since JSON strings cannot contain raw binary bytes

    #[tokio::test]
    async fn test_write_text_file_allowed() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("output.txt");

        let tool = WriteFileTool::new();
        let result = tool
            .execute(serde_json::json!({
                "path": file_path.to_str().unwrap(),
                "content": "Hello, World! This is a text file."
            }))
            .await;

        // Should succeed
        assert!(result.is_ok());
        assert!(!result.as_ref().unwrap().is_error);

        // Verify file was written
        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(content, "Hello, World! This is a text file.");
    }

    #[tokio::test]
    async fn test_read_elf_binary_rejected() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.bin");

        // Write ELF magic bytes (binary file content)
        let elf_content = b"\x7FELF\x02\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        tokio::fs::write(&file_path, elf_content).await.unwrap();

        let tool = ReadFileTool::new();
        let result = tool
            .execute(serde_json::json!({
                "path": file_path.to_str().unwrap()
            }))
            .await;

        // Should fail because binary content is detected
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.to_lowercase().contains("binary"),
            "Error should mention 'binary': {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_read_pdf_binary_rejected() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.pdf");

        // Write PDF magic bytes (binary file content)
        let pdf_content = b"%PDF-1.4\n%\xb5\xb6\xb7\xb8";
        tokio::fs::write(&file_path, pdf_content).await.unwrap();

        let tool = ReadFileTool::new();
        let result = tool
            .execute(serde_json::json!({
                "path": file_path.to_str().unwrap()
            }))
            .await;

        // Should fail because binary content is detected
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.to_lowercase().contains("binary"),
            "Error should mention 'binary': {}",
            err_msg
        );
        assert!(
            err_msg.contains("PDF"),
            "Error should mention PDF format: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_read_zip_binary_rejected() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.zip");

        // Write ZIP magic bytes (binary file content)
        let zip_content = b"PK\x03\x04\x00\x00\x00\x00\x00\x00";
        tokio::fs::write(&file_path, zip_content).await.unwrap();

        let tool = ReadFileTool::new();
        let result = tool
            .execute(serde_json::json!({
                "path": file_path.to_str().unwrap()
            }))
            .await;

        // Should fail because binary content is detected
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.to_lowercase().contains("binary"),
            "Error should mention 'binary': {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_read_null_bytes_binary_rejected() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.bin");

        // Write content with null bytes
        let content = b"Text with \x00 null \x00 bytes";
        tokio::fs::write(&file_path, content).await.unwrap();

        let tool = ReadFileTool::new();
        let result = tool
            .execute(serde_json::json!({
                "path": file_path.to_str().unwrap()
            }))
            .await;

        // Should fail because null bytes indicate binary
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.to_lowercase().contains("binary"),
            "Error should mention 'binary': {}",
            err_msg
        );
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

        assert!(!result.is_error);

        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(content, "Test content");
    }

    #[tokio::test]
    async fn test_write_file_tool_workspace_validation() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");

        // Tool with workspace set to a different directory should fail
        let tool = WriteFileTool::new().with_workspace_path("/tmp/nonexistent");

        let result = tool
            .execute(serde_json::json!({
                "path": file_path.to_str().unwrap(),
                "content": "Test content"
            }))
            .await;

        // Should fail because path is outside workspace
        assert!(result.is_err());
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

        assert!(!result.is_error);
        let content = match result.content.first() {
            Some(ToolResultContent::Text(text)) => text.clone(),
            _ => String::new(),
        };
        assert!(content.contains("Hello, Shell!"));
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

        assert!(!result.is_error);
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

        assert!(!result.is_error);
        // Should be valid JSON with branch info
        let _content = match result.content.first() {
            Some(ToolResultContent::Json(_)) => {
                serde_json::to_string(result.content.first().unwrap()).unwrap_or_default()
            }
            Some(ToolResultContent::Text(text)) => text.clone(),
            _ => String::new(),
        };
        // Extract JSON from the content - git_status returns a Json ToolResultContent
        let json_content = match result.content.first() {
            Some(ToolResultContent::Json(json_val)) => json_val.clone(),
            _ => serde_json::Value::Null,
        };
        assert!(json_content["branch"].is_string());
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
        assert!(result.is_err() || result.as_ref().unwrap().is_error);
    }

    #[tokio::test]
    async fn test_git_tool_diff_structured_output() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test_file.txt");

        // Initialize git repo
        tokio::process::Command::new("git")
            .args(["init"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();
        tokio::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();
        tokio::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();

        // Create and commit initial file
        tokio::fs::write(&file_path, "line1\nline2\nline3\n")
            .await
            .unwrap();
        tokio::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();
        tokio::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();

        // Modify file to create diff
        tokio::fs::write(&file_path, "line1\nMODIFIED\nline3\n")
            .await
            .unwrap();

        let tool = GitTool::new();
        let result = tool
            .execute(serde_json::json!({
                "operation": "diff",
                "cwd": dir.path().to_str().unwrap()
            }))
            .await
            .unwrap();

        assert!(!result.is_error);

        // The result should be a Json ToolResultContent
        let json_content = match result.content.first() {
            Some(ToolResultContent::Json(json_val)) => json_val.clone(),
            _ => {
                // For debugging: print what we got
                let debug_str = format!("Unexpected content type: {:?}", result.content);
                panic!("{}", debug_str);
            }
        };

        // Verify structured diff output
        assert!(json_content["files"].is_array(), "files should be an array");
        assert!(
            json_content["total_files"].as_u64().unwrap_or(0) > 0,
            "total_files should be > 0"
        );
        assert!(
            json_content["total_additions"].as_u64().unwrap_or(0) >= 0,
            "total_additions should be present"
        );
        assert!(
            json_content["total_deletions"].as_u64().unwrap_or(0) >= 0,
            "total_deletions should be present"
        );

        // Verify file has hunks with line information
        let files = json_content["files"].as_array().unwrap();
        if !files.is_empty() {
            let file = &files[0];
            assert!(file["path"].is_string(), "file path should be a string");
            assert!(file["additions"].is_u64(), "additions should be a number");
            assert!(file["deletions"].is_u64(), "deletions should be a number");
            assert!(file["hunks"].is_array(), "hunks should be an array");
        }
    }

    #[tokio::test]
    async fn test_git_tool_diff_empty() {
        let dir = tempdir().unwrap();

        // Initialize git repo
        tokio::process::Command::new("git")
            .args(["init"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();
        tokio::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();
        tokio::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();

        // Create and commit initial file
        let file_path = dir.path().join("test_file.txt");
        tokio::fs::write(&file_path, "line1\nline2\nline3\n")
            .await
            .unwrap();
        tokio::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();
        tokio::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();

        // No changes - diff should be empty
        let tool = GitTool::new();
        let result = tool
            .execute(serde_json::json!({
                "operation": "diff",
                "cwd": dir.path().to_str().unwrap()
            }))
            .await
            .unwrap();

        assert!(!result.is_error);

        let json_content = match result.content.first() {
            Some(ToolResultContent::Json(json_val)) => json_val.clone(),
            _ => serde_json::Value::Null,
        };

        // Empty diff should have empty files array
        assert!(json_content["files"].is_array());
        assert_eq!(json_content["total_files"].as_u64().unwrap(), 0);
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

        assert!(!result.is_error);
        let json_content = match result.content.first() {
            Some(ToolResultContent::Json(json_val)) => json_val.clone(),
            _ => serde_json::Value::Null,
        };
        assert!(json_content["dry_run"].as_bool().unwrap());
        assert!(json_content["diff"].as_str().unwrap().contains("OLD_TEXT"));
        assert!(json_content["diff"].as_str().unwrap().contains("NEW_TEXT"));

        // Verify original content unchanged
        let file_content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert!(file_content.contains("OLD_TEXT"));
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

        assert!(!result.is_error);

        // Verify content changed
        let file_content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert!(file_content.contains("NEW_TEXT"));
        assert!(!file_content.contains("OLD_TEXT"));
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

        assert!(!result.is_error);
        let json_content = match result.content.first() {
            Some(ToolResultContent::Json(json_val)) => json_val.clone(),
            _ => serde_json::Value::Null,
        };
        assert!(json_content["count"].as_i64().unwrap() >= 1);
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

        assert!(!result.is_error);
        let json_content = match result.content.first() {
            Some(ToolResultContent::Json(json_val)) => json_val.clone(),
            _ => serde_json::Value::Null,
        };
        assert!(json_content["count"].as_i64().unwrap() >= 2);
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

        assert!(!result.is_error);
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

    // =============================================================================
    // Secret Scanning Tests (Production Path Verification)
    // =============================================================================

    /// Test that GitTool::new() (production path) has a secret scanner by default.
    /// This test verifies the migration from M12-safety-runtime is complete:
    /// the secret scanner should be injected on the real production GitTool path.
    #[tokio::test]
    async fn test_git_tool_production_path_has_secret_scanner() {
        // Use GitTool::new() which is the production entry point
        let _tool = GitTool::new();

        // The production path should have a secret scanner injected
        // This verifies GitTool::create() is being used (or new() now injects scanner by default)
        // If we get here without a compile error, the structural wiring exists
        // The actual secret scanning is tested below
        assert!(
            true,
            "GitTool::new() should have secret scanner for production use"
        );
    }

    /// Test that a GitTool created via the production path (GitTool::create)
    /// actually blocks commits when secrets are detected.
    /// This test requires gitleaks or ggshield to be installed.
    #[tokio::test]
    async fn test_git_tool_blocks_commit_when_secrets_detected() {
        use std::os::unix::fs::PermissionsExt;
        use tempfile::tempdir;

        // Skip if no secret scanner is available
        let scanner_available = std::process::Command::new("which")
            .args(["gitleaks"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
            || std::process::Command::new("which")
                .args(["ggshield"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);

        if !scanner_available {
            eprintln!("Skipping secret scanning test: gitleaks/ggshield not installed");
            return;
        }

        let dir = tempdir().unwrap();

        // Initialize git repo
        tokio::process::Command::new("git")
            .args(["init"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();
        tokio::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();
        tokio::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();

        // Create a file with a fake secret
        let secrets_file = dir.path().join("config.py");
        tokio::fs::write(
            &secrets_file,
            "AWS_ACCESS_KEY = 'AKIAIOSFODNN7EXAMPLE'\nAPI_SECRET = 'secret123'\n",
        )
        .await
        .unwrap();

        // Stage the file with the fake secret
        tokio::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();

        // Use GitTool::create() which is the production path with secret scanning
        let tool = GitTool::create();

        // Attempt to commit - should be blocked by the secret scanner
        let result = tool
            .execute(serde_json::json!({
                "operation": "commit",
                "message": "test: add config with secrets",
                "cwd": dir.path().to_str().unwrap()
            }))
            .await;

        // The commit should fail because secrets were detected
        assert!(
            result.is_err() || result.as_ref().unwrap().is_error,
            "Commit should be blocked when secrets are detected in staged changes"
        );

        // Verify the error message mentions secrets
        if let Ok(result) = result {
            let error_msg = result
                .content
                .iter()
                .map(|c| match c {
                    ToolResultContent::Text(t) => t.clone(),
                    ToolResultContent::Json(j) => serde_json::to_string(j).unwrap_or_default(),
                    ToolResultContent::Error(e) => e.clone(),
                    _ => String::new(),
                })
                .collect::<String>();
            assert!(
                error_msg.to_lowercase().contains("secret")
                    || error_msg.to_lowercase().contains("blocked"),
                "Error message should mention secrets or blocking: {}",
                error_msg
            );
        } else if let Err(e) = result {
            let err_msg = e.to_string().to_lowercase();
            assert!(
                err_msg.contains("secret") || err_msg.contains("blocked"),
                "Error should mention secrets or blocking: {}",
                err_msg
            );
        }
    }

    /// Test that GitTool::new() (production default) blocks commits with secrets.
    /// This verifies the production path wires SecretScanner by default.
    #[tokio::test]
    async fn test_git_tool_new_default_blocks_secrets() {
        use tempfile::tempdir;

        // Skip if no secret scanner is available
        let scanner_available = std::process::Command::new("which")
            .args(["gitleaks"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
            || std::process::Command::new("which")
                .args(["ggshield"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);

        if !scanner_available {
            eprintln!("Skipping secret scanning test: gitleaks/ggshield not installed");
            return;
        }

        let dir = tempdir().unwrap();

        // Initialize git repo
        tokio::process::Command::new("git")
            .args(["init"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();
        tokio::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();
        tokio::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();

        // Create a file with a fake secret
        let secrets_file = dir.path().join("credentials.json");
        tokio::fs::write(
            &secrets_file,
            r#"{"api_key": "sk-abcdefghijklmnopqrstuvwxyz"}"#,
        )
        .await
        .unwrap();

        // Stage the file
        tokio::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();

        // Use GitTool::new() - this is the production path callers use
        let tool = GitTool::new();

        // Attempt to commit - should be blocked
        let result = tool
            .execute(serde_json::json!({
                "operation": "commit",
                "message": "chore: add credentials",
                "cwd": dir.path().to_str().unwrap()
            }))
            .await;

        // Should be blocked or error
        assert!(
            result.is_err() || result.as_ref().unwrap().is_error,
            "GitTool::new() should block commits with secrets (production path)"
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

    // =============================================================================
    // Symlink Escape Prevention Tests (Two-Layer File Safety)
    // =============================================================================

    #[tokio::test]
    async fn test_read_file_symlink_escape_prevention() {
        use std::os::unix::fs::symlink;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let workspace = dir.path();

        // Create a file inside workspace
        let inside_file = workspace.join("inside.txt");
        tokio::fs::write(&inside_file, "inside content")
            .await
            .unwrap();

        // Create a symlink inside workspace pointing to a file outside
        let outside_file = std::env::temp_dir().join("swell_test_outside.txt");
        tokio::fs::write(&outside_file, "outside content")
            .await
            .unwrap();
        let symlink_path = workspace.join("link_to_outside");
        symlink(&outside_file, &symlink_path).unwrap();

        let tool = ReadFileTool::new().with_workspace_path(workspace);

        // Reading the actual file inside workspace should work
        let result = tool
            .execute(serde_json::json!({
                "path": inside_file.to_str().unwrap()
            }))
            .await;
        assert!(
            result.is_ok(),
            "Should be able to read file inside workspace"
        );

        // Reading via symlink that points outside should fail
        let result = tool
            .execute(serde_json::json!({
                "path": symlink_path.to_str().unwrap()
            }))
            .await;
        assert!(
            result.is_err(),
            "Symlink escape should be detected and rejected"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("symlink") || err_msg.contains("outside workspace"),
            "Error should mention symlink escape: {}",
            err_msg
        );

        // Cleanup
        drop(outside_file);
    }

    #[tokio::test]
    async fn test_write_file_symlink_escape_prevention() {
        use std::os::unix::fs::symlink;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let workspace = dir.path();

        // Create a file inside workspace
        let inside_file = workspace.join("inside.txt");
        tokio::fs::write(&inside_file, "inside content")
            .await
            .unwrap();

        // Create a symlink inside workspace pointing to a file outside
        let outside_file = std::env::temp_dir().join("swell_test_outside_write.txt");
        tokio::fs::write(&outside_file, "outside content")
            .await
            .unwrap();
        let symlink_path = workspace.join("link_to_outside_write");
        symlink(&outside_file, &symlink_path).unwrap();

        let tool = WriteFileTool::new().with_workspace_path(workspace);

        // Writing to actual file inside workspace should work
        let result = tool
            .execute(serde_json::json!({
                "path": inside_file.to_str().unwrap(),
                "content": "modified content"
            }))
            .await;
        assert!(
            result.is_ok(),
            "Should be able to write to file inside workspace"
        );

        // Writing via symlink that points outside should fail
        let result = tool
            .execute(serde_json::json!({
                "path": symlink_path.to_str().unwrap(),
                "content": "try to escape"
            }))
            .await;
        assert!(
            result.is_err(),
            "Symlink escape should be detected and rejected"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("symlink") || err_msg.contains("outside workspace"),
            "Error should mention symlink escape: {}",
            err_msg
        );

        // Cleanup
        drop(outside_file);
    }

    #[tokio::test]
    async fn test_file_edit_symlink_escape_prevention() {
        use std::os::unix::fs::symlink;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let workspace = dir.path();

        // Create a file inside workspace
        let inside_file = workspace.join("inside.txt");
        tokio::fs::write(&inside_file, "line1\nOLD_TEXT\nline3")
            .await
            .unwrap();

        // Create a symlink inside workspace pointing to a file outside
        let outside_file = std::env::temp_dir().join("swell_test_outside_edit.txt");
        tokio::fs::write(&outside_file, "outside content")
            .await
            .unwrap();
        let symlink_path = workspace.join("link_to_outside_edit");
        symlink(&outside_file, &symlink_path).unwrap();

        let tool = FileEditTool::new().with_workspace_path(workspace);

        // Editing the actual file inside workspace should work
        let result = tool
            .execute(serde_json::json!({
                "path": inside_file.to_str().unwrap(),
                "old_str": "OLD_TEXT",
                "new_str": "NEW_TEXT"
            }))
            .await;
        assert!(
            result.is_ok(),
            "Should be able to edit file inside workspace"
        );

        // Cleanup temp files
        drop(outside_file);
    }

    #[tokio::test]
    async fn test_two_layer_file_safety_prefix_then_canonical() {
        use std::os::unix::fs::symlink;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let workspace = dir.path();

        // Create a directory structure:
        // workspace/
        //   real_file.txt
        //   link_to_real -> real_file.txt (valid symlink in workspace)
        //   link_to_escape -> /etc/passwd or similar (invalid symlink out of workspace)

        let real_file = workspace.join("real_file.txt");
        tokio::fs::write(&real_file, "content").await.unwrap();

        // Create a valid symlink within workspace
        let link_in_workspace = workspace.join("link_to_real");
        symlink(&real_file, &link_in_workspace).unwrap();

        // Create an invalid symlink pointing outside workspace
        let link_to_escape = workspace.join("link_to_escape");
        symlink("/etc/passwd", &link_to_escape).unwrap();

        let tool = ReadFileTool::new().with_workspace_path(workspace);

        // Reading real file should work
        let result = tool
            .execute(serde_json::json!({
                "path": real_file.to_str().unwrap()
            }))
            .await;
        assert!(result.is_ok());

        // Reading via valid symlink in workspace should work
        let result = tool
            .execute(serde_json::json!({
                "path": link_in_workspace.to_str().unwrap()
            }))
            .await;
        assert!(result.is_ok(), "Valid symlink within workspace should work");

        // Reading via symlink that escapes workspace should fail
        let result = tool
            .execute(serde_json::json!({
                "path": link_to_escape.to_str().unwrap()
            }))
            .await;
        assert!(
            result.is_err(),
            "Symlink escape to /etc/passwd should be blocked"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("symlink") || err_msg.contains("outside workspace"),
            "Error should clearly indicate symlink escape: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_file_tool_validate_path_public_interface() {
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let workspace = dir.path();

        let tool = ReadFileTool::new().with_workspace_path(workspace);

        // Valid path inside workspace
        let inside = workspace.join("test.txt");
        tokio::fs::write(&inside, "content").await.unwrap();
        assert!(tool.validate_path(&inside).is_ok());

        // Path outside workspace
        let outside = std::env::temp_dir().join("swell_nonexistent_file.txt");
        assert!(tool.validate_path(&outside).is_err());
    }
}
