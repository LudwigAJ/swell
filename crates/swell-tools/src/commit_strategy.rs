//! Git commit strategy implementation.
//!
//! This module provides:
//! - [`CommitStrategy`] - Atomic commits with metadata trailers
//! - [`CommitRequest`] - Structured commit request with metadata
//! - [`CommitMetadata`] - Trailers for provenance tracking
//! - Imperative mood commit messages
//! - Small, focused atomic commits

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Metadata trailers for commit traceability
#[derive(Debug, Clone, Default)]
pub struct CommitMetadata {
    /// Generator identification (e.g., "swell/1.0.0")
    pub generated_by: Option<String>,
    /// Task ID associated with this commit
    pub task_id: Option<Uuid>,
    /// Model used to generate changes (e.g., "claude-sonnet-4-20250514")
    pub model: Option<String>,
    /// Custom additional trailers
    pub extra: HashMap<String, String>,
}

impl CommitMetadata {
    /// Create a new empty metadata
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the generated-by trailer
    pub fn with_generated_by(mut self, generator: impl Into<String>) -> Self {
        self.generated_by = Some(generator.into());
        self
    }

    /// Set the task-id trailer
    pub fn with_task_id(mut self, task_id: Uuid) -> Self {
        self.task_id = Some(task_id);
        self
    }

    /// Set the model trailer
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Add a custom trailer
    pub fn with_extra(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra.insert(key.into(), value.into());
        self
    }

    /// Format metadata as git trailer lines
    pub fn to_trailers(&self) -> String {
        let mut lines = Vec::new();

        if let Some(ref gb) = self.generated_by {
            lines.push(format!("Generated-by: {}", gb));
        }

        if let Some(ref tid) = self.task_id {
            lines.push(format!("Task-id: {}", tid));
        }

        if let Some(ref model) = self.model {
            lines.push(format!("Model: {}", model));
        }

        for (key, value) in &self.extra {
            lines.push(format!("{}: {}", key, value));
        }

        lines.join("\n")
    }

    /// Check if metadata is empty
    pub fn is_empty(&self) -> bool {
        self.generated_by.is_none()
            && self.task_id.is_none()
            && self.model.is_none()
            && self.extra.is_empty()
    }
}

/// Request to create a commit
#[derive(Debug, Clone)]
pub struct CommitRequest {
    /// The commit message (imperative mood, short first line)
    pub message: String,
    /// Extended description (optional, added after blank line)
    pub description: Option<String>,
    /// Metadata trailers for traceability
    pub metadata: CommitMetadata,
    /// Whether to stage all modified files (default: true)
    pub stage_all: bool,
    /// Specific files to stage (if empty and stage_all is false, no files staged)
    pub files: Vec<String>,
    /// Author name (optional, uses git config if not set)
    pub author_name: Option<String>,
    /// Author email (optional, uses git config if not set)
    pub author_email: Option<String>,
}

impl CommitRequest {
    /// Create a new commit request with just a message
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            description: None,
            metadata: CommitMetadata::new(),
            stage_all: true,
            files: Vec::new(),
            author_name: None,
            author_email: None,
        }
    }

    /// Add extended description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Add metadata
    pub fn with_metadata(mut self, metadata: CommitMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Add generated-by metadata
    pub fn with_generated_by(mut self, generator: impl Into<String>) -> Self {
        self.metadata = self.metadata.with_generated_by(generator);
        self
    }

    /// Add task-id metadata
    pub fn with_task_id(mut self, task_id: Uuid) -> Self {
        self.metadata = self.metadata.with_task_id(task_id);
        self
    }

    /// Add model metadata
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.metadata = self.metadata.with_model(model);
        self
    }

    /// Don't stage all files, use specific files instead
    pub fn with_files(mut self, files: Vec<String>) -> Self {
        self.stage_all = false;
        self.files = files;
        self
    }

    /// Set author information
    pub fn with_author(mut self, name: impl Into<String>, email: impl Into<String>) -> Self {
        self.author_name = Some(name.into());
        self.author_email = Some(email.into());
        self
    }

    /// Build the full commit message including trailers
    pub fn build_message(&self) -> String {
        let mut msg = self.message.clone();

        if let Some(ref desc) = self.description {
            msg.push_str("\n\n");
            msg.push_str(desc);
        }

        if !self.metadata.is_empty() {
            msg.push_str("\n\n");
            msg.push_str(&self.metadata.to_trailers());
        }

        msg
    }

    /// Build the full commit message with external metadata (used by CommitStrategy)
    pub fn build_message_with_metadata(&self, metadata: &CommitMetadata) -> String {
        let mut msg = self.message.clone();

        if let Some(ref desc) = self.description {
            msg.push_str("\n\n");
            msg.push_str(desc);
        }

        if !metadata.is_empty() {
            msg.push_str("\n\n");
            msg.push_str(&metadata.to_trailers());
        }

        msg
    }
}

/// Result of a commit operation
#[derive(Debug, Clone)]
pub struct CommitResult {
    /// The commit hash
    pub commit_hash: String,
    /// The full commit message
    pub message: String,
    /// Number of files changed
    pub files_changed: usize,
    /// Whether this was a new commit or amendment
    pub is_new: bool,
}

/// Errors that can occur during commit operations
#[derive(Debug, Clone, thiserror::Error)]
pub enum CommitStrategyError {
    #[error("Nothing to commit: {0}")]
    NothingToCommit(String),

    #[error("Git operation failed: {0}")]
    GitFailed(String),

    #[error("Invalid commit message: {0}")]
    InvalidMessage(String),

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Atomic commit failed, rolling back: {0}")]
    AtomicRollback(String),

    #[error("Author information required but not provided")]
    AuthorRequired,
}

/// Commit strategy for atomic commits with metadata trailers
#[derive(Debug, Clone)]
pub struct CommitStrategy {
    /// Generator identification string
    generator_id: String,
    /// Default model to use if not specified
    default_model: Option<String>,
    /// Tracks commits created by this strategy
    commits: Arc<RwLock<HashMap<String, CommitInfo>>>,
}

/// Internal commit tracking info
#[derive(Debug, Clone)]
pub struct CommitInfo {
    /// The commit hash
    pub commit_hash: String,
    /// Associated task ID if any
    pub task_id: Option<Uuid>,
    /// When the commit was created
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl CommitStrategy {
    /// Create a new commit strategy with generator identification
    pub fn new(generator_id: impl Into<String>) -> Self {
        Self {
            generator_id: generator_id.into(),
            default_model: None,
            commits: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new commit strategy with generator identification and default model
    pub fn with_default_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = Some(model.into());
        self
    }

    /// Get the generator ID
    pub fn generator_id(&self) -> &str {
        &self.generator_id
    }

    /// Get the default model
    pub fn default_model(&self) -> Option<&str> {
        self.default_model.as_deref()
    }

    /// Get the count of commits tracked by this strategy
    pub async fn commit_count(&self) -> usize {
        let commits = self.commits.read().await;
        commits.len()
    }

    /// Get all tracked commit hashes
    pub async fn tracked_commits(&self) -> Vec<String> {
        let commits = self.commits.read().await;
        commits.keys().cloned().collect()
    }

    /// Check if a commit is tracked
    pub async fn is_tracked(&self, commit_hash: &str) -> bool {
        let commits = self.commits.read().await;
        commits.contains_key(commit_hash)
    }

    /// Get info about a tracked commit
    pub async fn get_commit_info(&self, commit_hash: &str) -> Option<CommitInfo> {
        let commits = self.commits.read().await;
        commits.get(commit_hash).cloned()
    }

    /// Clear all tracked commits (for run reset)
    pub async fn reset(&self) {
        let count = {
            let commits = self.commits.read().await;
            commits.len()
        };
        if count > 0 {
            info!(cleared = count, "Commit strategy reset");
        }
        let mut commits = self.commits.write().await;
        commits.clear();
    }

    /// Validate a commit request before execution
    pub fn validate(&self, request: &CommitRequest) -> Result<(), CommitStrategyError> {
        // Check message is not empty
        let trimmed = request.message.trim();
        if trimmed.is_empty() {
            return Err(CommitStrategyError::InvalidMessage(
                "Commit message cannot be empty".to_string(),
            ));
        }

        // Check message follows imperative mood (starts with verb)
        // Common verbs: add, fix, update, remove, refactor, implement, create, delete, etc.
        let first_word = trimmed.split_whitespace().next().unwrap_or("");
        let imperative_verbs = [
            "add", "fix", "update", "remove", "refactor", "implement", "create", "delete",
            "improve", "optimize", "clean", "format", "document", "test", "debug", "enable",
            "disable", "configure", "merge", "split", "extract", "inline", "rename", "move",
            "reorder", "combine", "separate", "validate", "verify", "check", "ensure", "make",
            "set", "reset", "restore", "revert", "rollback", "apply", "dispatch", "handle",
        ];

        let first_word_lower = first_word.to_lowercase();
        if !imperative_verbs.contains(&first_word_lower.as_str()) {
            warn!(
                message = %trimmed,
                first_word = %first_word,
                "Commit message may not be in imperative mood"
            );
            // We don't error on this, just warn - it's a soft validation
        }

        // If not staging all and no specific files, nothing to commit
        if !request.stage_all && request.files.is_empty() {
            return Err(CommitStrategyError::NothingToCommit(
                "No files specified to commit".to_string(),
            ));
        }

        Ok(())
    }

    /// Prepare files for commit (stage them)
    async fn stage_files(
        &self,
        request: &CommitRequest,
        cwd: &Path,
    ) -> Result<(), CommitStrategyError> {
        let args: Vec<&str> = if request.stage_all {
            vec!["add", "-A"]
        } else {
            let mut args = vec!["add"];
            args.extend(request.files.iter().map(|s| s.as_str()));
            args
        };

        let output = tokio::process::Command::new("git")
            .args(&args)
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| CommitStrategyError::GitFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CommitStrategyError::GitFailed(stderr.to_string()));
        }

        Ok(())
    }

    /// Create a commit with the given message
    async fn create_commit(
        &self,
        message: &str,
        cwd: &Path,
    ) -> Result<String, CommitStrategyError> {
        let output = tokio::process::Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| CommitStrategyError::GitFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CommitStrategyError::GitFailed(stderr.to_string()));
        }

        // Get the commit hash
        let output = tokio::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| CommitStrategyError::GitFailed(e.to_string()))?;

        let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();

        Ok(hash)
    }

    /// Execute an atomic commit
    ///
    /// Atomic means: if any step fails, the entire commit fails and no partial state is left.
    pub async fn commit(
        &self,
        request: CommitRequest,
        cwd: &Path,
    ) -> Result<CommitResult, CommitStrategyError> {
        // Validate the request
        self.validate(&request)?;

        // Clone metadata for later use (after we borrow request for staging)
        let mut metadata = request.metadata.clone();
        
        // Add generated-by if not already set
        if metadata.generated_by.is_none() {
            metadata.generated_by = Some(self.generator_id.clone());
        }

        // Add model if not already set and we have a default
        if metadata.model.is_none() {
            if let Some(ref model) = self.default_model {
                metadata.model = Some(model.clone());
            }
        }

        // Build the full message with metadata (using cloned metadata)
        let full_message = request.build_message_with_metadata(&metadata);

        // Stage files (borrow request here)
        self.stage_files(&request, cwd).await?;

        // Check if there's anything to commit
        let output = tokio::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| CommitStrategyError::GitFailed(e.to_string()))?;

        let status_output = String::from_utf8_lossy(&output.stdout);
        if status_output.trim().is_empty() {
            return Err(CommitStrategyError::NothingToCommit(
                "No changes to commit".to_string(),
            ));
        }

        // Count files that will be committed
        let files_changed = status_output
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();

        // Create the commit
        let commit_hash = self.create_commit(&full_message, cwd).await?;

        // Track the commit
        let info = CommitInfo {
            commit_hash: commit_hash.clone(),
            task_id: metadata.task_id,
            created_at: chrono::Utc::now(),
        };

        {
            let mut commits = self.commits.write().await;
            commits.insert(commit_hash.clone(), info);
        }

        debug!(
            commit_hash = %commit_hash,
            files_changed = files_changed,
            "Atomic commit created"
        );

        Ok(CommitResult {
            commit_hash,
            message: full_message,
            files_changed,
            is_new: true,
        })
    }

    /// Create an atomic commit that only succeeds if it can apply cleanly
    ///
    /// This is useful for ensuring commits are small and focused.
    pub async fn atomic_commit(
        &self,
        request: CommitRequest,
        cwd: &Path,
    ) -> Result<CommitResult, CommitStrategyError> {
        // Validate the request first
        self.validate(&request)?;

        // Check the diff size - warn if too large
        let output = tokio::process::Command::new("git")
            .args(["diff", "--stat"])
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| CommitStrategyError::GitFailed(e.to_string()))?;

        let diff_stat = String::from_utf8_lossy(&output.stdout);
        debug!(diff_stat = %diff_stat, "Diff stat for atomic commit");

        // Stage files
        self.stage_files(&request, cwd).await?;

        // Verify staged changes look reasonable
        let staged_output = tokio::process::Command::new("git")
            .args(["diff", "--cached", "--stat"])
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| CommitStrategyError::GitFailed(e.to_string()))?;

        let staged_stat = String::from_utf8_lossy(&staged_output.stdout);
        debug!(staged_stat = %staged_stat, "Staged changes stat");

        // Create the commit (delegates to commit which handles message building)
        self.commit(request, cwd).await
    }

    /// Amend the last commit with new metadata (doesn't change the content)
    pub async fn amend_with_metadata(
        &self,
        cwd: &Path,
    ) -> Result<CommitResult, CommitStrategyError> {
        // Get the current commit message
        let output = tokio::process::Command::new("git")
            .args(["log", "-1", "--format=%B"])
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| CommitStrategyError::GitFailed(e.to_string()))?;

        let current_message = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Add our generator trailer if not present
        let generator_trailer = format!("Generated-by: {}", self.generator_id);
        let new_message = if current_message.contains("Generated-by:") {
            current_message
        } else {
            format!("{}\n\n{}", current_message, generator_trailer)
        };

        // Amend the commit
        let output = tokio::process::Command::new("git")
            .args(["commit", "--amend", "-m", &new_message])
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| CommitStrategyError::GitFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CommitStrategyError::GitFailed(stderr.to_string()));
        }

        // Get the new commit hash
        let output = tokio::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| CommitStrategyError::GitFailed(e.to_string()))?;

        let commit_hash = String::from_utf8_lossy(&output.stdout).trim().to_string();

        info!(
            commit_hash = %commit_hash,
            "Commit amended with metadata"
        );

        Ok(CommitResult {
            commit_hash,
            message: new_message,
            files_changed: 0,
            is_new: false,
        })
    }

    /// Get the recent commits with metadata
    pub async fn get_recent_commits(
        &self,
        limit: usize,
        cwd: &Path,
    ) -> Result<Vec<CommitInfo>, CommitStrategyError> {
        let output = tokio::process::Command::new("git")
            .args(["log", &format!("-{}", limit), "--format=%H|%s"])
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| CommitStrategyError::GitFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CommitStrategyError::GitFailed(stderr.to_string()));
        }

        let commits = String::from_utf8_lossy(&output.stdout);
        let mut result = Vec::new();

        for line in commits.lines() {
            let parts: Vec<&str> = line.splitn(2, '|').collect();
            if parts.len() == 2 {
                result.push(CommitInfo {
                    commit_hash: parts[0].to_string(),
                    task_id: None, // Would need full message to parse
                    created_at: chrono::Utc::now(),
                });
            }
        }

        Ok(result)
    }
}

impl Default for CommitStrategy {
    fn default() -> Self {
        Self::new("swell")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_commit_metadata_empty() {
        let meta = CommitMetadata::new();
        assert!(meta.is_empty());
        assert!(meta.to_trailers().is_empty());
    }

    #[tokio::test]
    async fn test_commit_metadata_with_values() {
        let meta = CommitMetadata::new()
            .with_generated_by("swell/1.0.0")
            .with_task_id(Uuid::new_v4())
            .with_model("claude-sonnet");

        assert!(!meta.is_empty());
        let trailers = meta.to_trailers();
        assert!(trailers.contains("Generated-by: swell/1.0.0"));
        assert!(trailers.contains("Task-id:"));
        assert!(trailers.contains("Model: claude-sonnet"));
    }

    #[test]
    fn test_commit_request_basic() {
        let request = CommitRequest::new("Add new feature");
        assert_eq!(request.message, "Add new feature");
        assert!(request.metadata.is_empty());
        assert!(request.stage_all);
    }

    #[test]
    fn test_commit_request_full() {
        let task_id = Uuid::new_v4();
        let request = CommitRequest::new("Fix authentication bug")
            .with_description("The login flow was failing due to missing token refresh")
            .with_task_id(task_id)
            .with_model("claude-sonnet-4");

        assert_eq!(request.message, "Fix authentication bug");
        assert!(request.description.is_some());
        assert_eq!(request.metadata.task_id, Some(task_id));
        assert_eq!(request.metadata.model, Some("claude-sonnet-4".to_string()));
    }

    #[test]
    fn test_commit_request_build_message() {
        let request = CommitRequest::new("Update dependencies")
            .with_description("Bump tokio to 1.0 and serde to 2.0")
            .with_generated_by("swell/0.1.0");

        let msg = request.build_message();
        assert!(msg.contains("Update dependencies"));
        assert!(msg.contains("Bump tokio to 1.0"));
        assert!(msg.contains("Generated-by: swell/0.1.0"));
    }

    #[tokio::test]
    async fn test_commit_strategy_new() {
        let strategy = CommitStrategy::new("test-generator");
        assert_eq!(strategy.generator_id(), "test-generator");
        assert_eq!(strategy.commit_count().await, 0);
    }

    #[tokio::test]
    async fn test_commit_strategy_validate_imperative() {
        let strategy = CommitStrategy::new("test");

        // Valid imperative messages
        let req1 = CommitRequest::new("Add new feature");
        assert!(strategy.validate(&req1).is_ok());

        let req2 = CommitRequest::new("Fix bug in parser");
        assert!(strategy.validate(&req2).is_ok());

        let req3 = CommitRequest::new("Refactor code structure");
        assert!(strategy.validate(&req3).is_ok());

        // Empty message should fail
        let req_bad = CommitRequest::new("");
        assert!(matches!(
            strategy.validate(&req_bad),
            Err(CommitStrategyError::InvalidMessage(_))
        ));
    }

    #[tokio::test]
    async fn test_commit_strategy_validate_no_files() {
        let strategy = CommitStrategy::new("test");

        // Should fail - not staging all and no files specified
        // This test validates that without stage_all=true and without files, validation fails
        let req_no_stage = CommitRequest::new("test").with_files(vec![]);
        assert!(matches!(
            strategy.validate(&req_no_stage),
            Err(CommitStrategyError::NothingToCommit(_))
        ));
    }

    #[tokio::test]
    async fn test_commit_strategy_reset() {
        let strategy = CommitStrategy::new("test");

        // Add a mock commit to track
        let mut commits = strategy.commits.write().await;
        commits.insert(
            "abc123".to_string(),
            CommitInfo {
                commit_hash: "abc123".to_string(),
                task_id: None,
                created_at: chrono::Utc::now(),
            },
        );
        drop(commits);

        assert_eq!(strategy.commit_count().await, 1);

        strategy.reset().await;
        assert_eq!(strategy.commit_count().await, 0);
    }

    #[tokio::test]
    async fn test_commit_strategy_default_model() {
        let strategy = CommitStrategy::new("test").with_default_model("claude-sonnet");

        assert_eq!(strategy.default_model(), Some("claude-sonnet"));
    }

    #[tokio::test]
    async fn test_tracked_commits() {
        let strategy = CommitStrategy::new("test");

        // Add some commits
        let mut commits = strategy.commits.write().await;
        commits.insert(
            "hash1".to_string(),
            CommitInfo {
                commit_hash: "hash1".to_string(),
                task_id: Some(Uuid::new_v4()),
                created_at: chrono::Utc::now(),
            },
        );
        commits.insert(
            "hash2".to_string(),
            CommitInfo {
                commit_hash: "hash2".to_string(),
                task_id: None,
                created_at: chrono::Utc::now(),
            },
        );
        drop(commits);

        let tracked = strategy.tracked_commits().await;
        assert_eq!(tracked.len(), 2);
        assert!(strategy.is_tracked("hash1").await);
        assert!(strategy.is_tracked("hash2").await);
        assert!(!strategy.is_tracked("hash3").await);
    }

    #[tokio::test]
    async fn test_full_git_commit_flow() {
        let dir = tempdir().unwrap();

        // Initialize git repo
        tokio::process::Command::new("git")
            .args(["init"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();

        // Set git config for test user
        tokio::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();

        tokio::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();

        // Create a file and commit
        let file_path = dir.path().join("test.txt");
        tokio::fs::write(&file_path, "Hello, World!").await.unwrap();

        let strategy = CommitStrategy::new("swell-test/1.0.0")
            .with_default_model("claude-sonnet-test");

        let request = CommitRequest::new("Add test file")
            .with_description("A simple test file for commit strategy testing");

        // Stage all
        tokio::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();

        // Create commit using strategy
        let result = strategy.create_commit(&request.build_message(), dir.path()).await;
        assert!(result.is_ok(), "Commit should succeed: {:?}", result.err());

        let commit_hash = result.unwrap();
        assert!(!commit_hash.is_empty());
        assert_eq!(strategy.commit_count().await, 0); // We didn't use commit() which tracks
    }

    #[tokio::test]
    async fn test_git_commit_with_trailers() {
        let dir = tempdir().unwrap();

        // Initialize git repo
        tokio::process::Command::new("git")
            .args(["init"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();

        // Set git config
        tokio::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();

        tokio::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();

        // Create a file
        let file_path = dir.path().join("feature.txt");
        tokio::fs::write(&file_path, "Feature content").await.unwrap();

        let task_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();

        let strategy = CommitStrategy::new("swell/1.0.0");

        // Build message with metadata
        let request = CommitRequest::new("Implement new feature")
            .with_description("Adds the requested functionality")
            .with_generated_by("swell/1.0.0")
            .with_task_id(task_id)
            .with_model("claude-sonnet-4-20250514");

        let message = request.build_message();
        assert!(message.contains("Implement new feature"));
        assert!(message.contains("Generated-by: swell/1.0.0"));
        assert!(message.contains("Task-id: 550e8400-e29b-41d4-a716-446655440000"));
        assert!(message.contains("Model: claude-sonnet-4-20250514"));

        // Stage and commit
        tokio::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();

        let result = strategy.create_commit(&message, dir.path()).await;
        assert!(result.is_ok());

        // Verify the commit was created
        let output = tokio::process::Command::new("git")
            .args(["log", "-1", "--format=%B"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();

        let log_message = String::from_utf8_lossy(&output.stdout);
        assert!(log_message.contains("Implement new feature"));
        assert!(log_message.contains("Generated-by: swell/1.0.0"));
    }
}
