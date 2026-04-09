//! Git Pull Request creation with metadata, evidence, and labels.
//!
//! This module provides:
//! - [`PrCreator`] - Creates PRs with diff, task description, validation results
//! - [`PrMetadata`] - Structured metadata for PR creation
//! - [`EvidenceSummary`] - Summary of validation evidence for PR body
//! - Label management for tracking
//!
//! PRs created by this module include:
//! - Task description and rationale
//! - Diff summary of changes
//! - Validation evidence summary
//! - Labels for tracking (task type, status, milestone)

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Label categories for PR tracking
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PrLabel {
    /// Task type labels
    TypeFeature,
    TypeBugfix,
    TypeRefactor,
    TypeDocs,
    TypeTest,
    /// Status labels
    StatusReady,
    StatusDraft,
    StatusWip,
    /// Milestone labels (dynamically generated)
    Milestone(String),
    /// Priority labels
    PriorityHigh,
    PriorityMedium,
    PriorityLow,
    /// Validation status
    Validated,
    NeedsReview,
}

impl PrLabel {
    /// Get the string representation for git tag/label
    pub fn as_str(&self) -> String {
        match self {
            PrLabel::TypeFeature => "type:feature".to_string(),
            PrLabel::TypeBugfix => "type:bugfix".to_string(),
            PrLabel::TypeRefactor => "type:refactor".to_string(),
            PrLabel::TypeDocs => "type:docs".to_string(),
            PrLabel::TypeTest => "type:test".to_string(),
            PrLabel::StatusReady => "status:ready".to_string(),
            PrLabel::StatusDraft => "status:draft".to_string(),
            PrLabel::StatusWip => "status:wip".to_string(),
            PrLabel::Milestone(m) => format!("milestone:{}", m),
            PrLabel::PriorityHigh => "priority:high".to_string(),
            PrLabel::PriorityMedium => "priority:medium".to_string(),
            PrLabel::PriorityLow => "priority:low".to_string(),
            PrLabel::Validated => "validated".to_string(),
            PrLabel::NeedsReview => "needs-review".to_string(),
        }
    }
}

/// Configuration for PR creation
#[derive(Debug, Clone)]
pub struct PrCreatorConfig {
    /// Default base branch for PRs (default: "main")
    pub default_base_branch: String,
    /// Whether to require validation evidence before PR creation
    pub require_validation: bool,
    /// Whether to add task metadata as PR body section
    pub include_task_metadata: bool,
    /// Whether to include diff stats in PR body
    pub include_diff_stats: bool,
    /// Whether to allow draft PRs
    pub allow_draft: bool,
}

impl Default for PrCreatorConfig {
    fn default() -> Self {
        Self {
            default_base_branch: "main".to_string(),
            require_validation: true,
            include_task_metadata: true,
            include_diff_stats: true,
            allow_draft: true,
        }
    }
}

/// Metadata for PR creation
#[derive(Debug, Clone)]
pub struct PrMetadata {
    /// Task ID associated with this PR
    pub task_id: Option<Uuid>,
    /// Task description (used as PR title if not specified)
    pub task_description: String,
    /// PR title (defaults to task_description if not set)
    pub title: Option<String>,
    /// Extended description/rationale for the changes
    pub description: Option<String>,
    /// Base branch to create PR against
    pub base_branch: Option<String>,
    /// Labels to apply to the PR
    pub labels: Vec<PrLabel>,
    /// Custom metadata key-value pairs
    pub metadata: HashMap<String, String>,
}

impl PrMetadata {
    /// Create new PR metadata with task description
    pub fn new(task_description: impl Into<String>) -> Self {
        Self {
            task_id: None,
            task_description: task_description.into(),
            title: None,
            description: None,
            base_branch: None,
            labels: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    /// Set the task ID
    pub fn with_task_id(mut self, task_id: Uuid) -> Self {
        self.task_id = Some(task_id);
        self
    }

    /// Set the PR title
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the extended description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the base branch
    pub fn with_base_branch(mut self, base_branch: impl Into<String>) -> Self {
        self.base_branch = Some(base_branch.into());
        self
    }

    /// Add a label
    pub fn with_label(mut self, label: PrLabel) -> Self {
        self.labels.push(label);
        self
    }

    /// Add multiple labels
    pub fn with_labels(mut self, labels: impl IntoIterator<Item = PrLabel>) -> Self {
        self.labels.extend(labels);
        self
    }

    /// Add metadata key-value pair
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Get the PR title (title or task_description)
    pub fn pr_title(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.task_description)
    }

    /// Get the base branch
    pub fn base_branch<'a>(&'a self, default: &'a str) -> &'a str {
        self.base_branch.as_deref().unwrap_or(default)
    }
}

/// Evidence summary for PR body
#[derive(Debug, Clone)]
pub struct EvidenceSummary {
    /// Total tests run
    pub tests_passed: usize,
    /// Tests that failed
    pub tests_failed: usize,
    /// Tests skipped
    pub tests_skipped: usize,
    /// Lint passed
    pub lint_passed: bool,
    /// Security scan passed
    pub security_passed: bool,
    /// Overall validation outcome
    pub overall_passed: bool,
    /// Confidence score (0.0 - 1.0)
    pub confidence_score: f64,
    /// Evidence pack ID if available
    pub evidence_pack_id: Option<Uuid>,
    /// Additional notes
    pub notes: Option<String>,
}

impl EvidenceSummary {
    /// Create a new evidence summary
    pub fn new() -> Self {
        Self {
            tests_passed: 0,
            tests_failed: 0,
            tests_skipped: 0,
            lint_passed: true,
            security_passed: true,
            overall_passed: true,
            confidence_score: 1.0,
            evidence_pack_id: None,
            notes: None,
        }
    }

    /// Create from evidence pack if available (using swell-validation types)
    pub fn from_evidence_pack(passed: bool, confidence: f64) -> Self {
        Self {
            tests_passed: 0,
            tests_failed: 0,
            tests_skipped: 0,
            lint_passed: passed,
            security_passed: passed,
            overall_passed: passed,
            confidence_score: confidence,
            evidence_pack_id: None,
            notes: None,
        }
    }

    /// Format as markdown for PR body
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        md.push_str("## Validation Evidence\n\n");

        // Overall status
        let status_emoji = if self.overall_passed {
            "✅"
        } else {
            "❌"
        };
        let status_text = if self.overall_passed { "PASSED" } else { "FAILED" };
        md.push_str(&format!("**Overall: {} {}**\n\n", status_emoji, status_text));

        // Test results
        md.push_str("### Tests\n\n");
        md.push_str(
            "| Passed | Failed | Skipped |\n|--------|--------|---------|\n",
        );
        md.push_str(&format!(
            "| {} | {} | {} |\n\n",
            self.tests_passed, self.tests_failed, self.tests_skipped
        ));

        // Other checks
        md.push_str("### Checks\n\n");
        md.push_str(&format!(
            "- **Lint**: {}\n",
            if self.lint_passed { "✅ Passed" } else { "❌ Failed" }
        ));
        md.push_str(&format!(
            "- **Security**: {}\n",
            if self.security_passed {
                "✅ Passed"
            } else {
                "❌ Failed"
            }
        ));
        md.push_str(&format!(
            "- **Confidence**: {:.0}%\n",
            self.confidence_score * 100.0
        ));

        if let Some(pack_id) = self.evidence_pack_id {
            md.push_str(&format!("\n*Evidence pack ID: {}*\n", pack_id));
        }

        if let Some(ref notes) = self.notes {
            md.push_str(&format!("\n**Notes**: {}\n", notes));
        }

        md
    }
}

impl Default for EvidenceSummary {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of PR creation
#[derive(Debug, Clone)]
pub struct PrResult {
    /// PR number (if created successfully)
    pub pr_number: Option<u32>,
    /// PR URL (if created successfully)
    pub pr_url: Option<String>,
    /// Whether PR was created
    pub created: bool,
    /// Whether it's a draft PR
    pub is_draft: bool,
    /// Branch name that was created
    pub branch_name: String,
    /// Base branch
    pub base_branch: String,
    /// Labels that were applied
    pub labels_applied: Vec<String>,
    /// Error message if creation failed
    pub error: Option<String>,
}

/// Errors from PR creation
#[derive(Debug, Clone, thiserror::Error)]
pub enum PrCreationError {
    #[error("Git operation failed: {0}")]
    GitFailed(String),

    #[error("No remote configured for repository")]
    NoRemote,

    #[error("Branch '{0}' has no commits to create PR from")]
    NoCommits(String),

    #[error("Failed to create PR: {0}")]
    FailedToCreate(String),

    #[error("No diff to create PR with")]
    NoDiff,

    #[error("Validation required but evidence not provided")]
    ValidationRequired,
}

/// PR Creator for git repositories
#[derive(Debug, Clone)]
pub struct PrCreator {
    config: PrCreatorConfig,
    /// Tracks created PRs in this session
    created_prs: Arc<RwLock<HashMap<String, PrResult>>>,
}

impl PrCreator {
    /// Create a new PR creator with default configuration
    pub fn new() -> Self {
        Self {
            config: PrCreatorConfig::default(),
            created_prs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new PR creator with custom configuration
    pub fn with_config(config: PrCreatorConfig) -> Self {
        Self {
            config,
            created_prs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get the current configuration
    pub fn config(&self) -> &PrCreatorConfig {
        &self.config
    }

    /// Get the number of PRs created by this creator
    pub async fn created_count(&self) -> usize {
        let prs = self.created_prs.read().await;
        prs.len()
    }

    /// Get all created PR results
    pub async fn created_prs(&self) -> Vec<PrResult> {
        let prs = self.created_prs.read().await;
        prs.values().cloned().collect()
    }

    /// Get a specific PR result by branch name
    pub async fn get_pr(&self, branch_name: &str) -> Option<PrResult> {
        let prs = self.created_prs.read().await;
        prs.get(branch_name).cloned()
    }

    /// Get the current branch name
    pub async fn get_current_branch(&self, cwd: &Path) -> Result<String, PrCreationError> {
        let output = tokio::process::Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| PrCreationError::GitFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PrCreationError::GitFailed(stderr.to_string()));
        }

        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if branch.is_empty() {
            return Err(PrCreationError::GitFailed(
                "Not on a branch (detached HEAD)".to_string(),
            ));
        }

        Ok(branch)
    }

    /// Get the diff summary for the current branch
    pub async fn get_diff_summary(&self, cwd: &Path) -> Result<String, PrCreationError> {
        let output = tokio::process::Command::new("git")
            .args(["diff", "--stat", "HEAD"])
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| PrCreationError::GitFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PrCreationError::GitFailed(stderr.to_string()));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Get the full diff for the branch
    pub async fn get_full_diff(&self, cwd: &Path, base_branch: &str) -> Result<String, PrCreationError> {
        let output = tokio::process::Command::new("git")
            .args(["diff", &format!("{}..HEAD", base_branch)])
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| PrCreationError::GitFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PrCreationError::GitFailed(stderr.to_string()));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Get remote URL for the repository
    pub async fn get_remote_url(&self, cwd: &Path) -> Result<String, PrCreationError> {
        let output = tokio::process::Command::new("git")
            .args(["remote", "get-url", "origin"])
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| PrCreationError::GitFailed(e.to_string()))?;

        if !output.status.success() {
            return Err(PrCreationError::NoRemote);
        }

        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(url)
    }

    /// Check if gh CLI is available
    pub fn is_gh_available(&self) -> bool {
        which::which("gh").is_ok()
    }

    /// Create a PR using gh CLI
    pub async fn create_pr_with_gh(
        &self,
        metadata: &PrMetadata,
        evidence: Option<&EvidenceSummary>,
        cwd: &Path,
    ) -> Result<PrResult, PrCreationError> {
        if !self.is_gh_available() {
            return Err(PrCreationError::FailedToCreate(
                "GitHub CLI (gh) is not installed".to_string(),
            ));
        }

        let current_branch = self.get_current_branch(cwd).await?;
        let base_branch = metadata.base_branch(&self.config.default_base_branch);

        // Build the PR body
        let mut body = String::new();

        // Add description if provided
        if let Some(ref desc) = metadata.description {
            body.push_str(desc);
            body.push_str("\n\n");
        }

        // Add task metadata
        if self.config.include_task_metadata {
            body.push_str("## Task Information\n\n");
            if let Some(task_id) = metadata.task_id {
                body.push_str(&format!("- **Task ID**: {}\n", task_id));
            }
            body.push_str(&format!("- **Branch**: {}\n", current_branch));
            body.push('\n');
        }

        // Add diff stats
        if self.config.include_diff_stats {
            match self.get_diff_summary(cwd).await {
                Ok(stats) => {
                    body.push_str("## Changes Summary\n\n");
                    body.push_str("```\n");
                    body.push_str(&stats);
                    body.push_str("```\n\n");
                }
                Err(e) => {
                    warn!(error = %e, "Could not get diff summary");
                }
            }
        }

        // Add evidence if provided
        if let Some(ev) = evidence {
            body.push_str(&ev.to_markdown());
            body.push('\n');
        }

        // Add custom metadata
        if !metadata.metadata.is_empty() {
            body.push_str("## Metadata\n\n");
            for (key, value) in &metadata.metadata {
                body.push_str(&format!("- **{}**: {}\n", key, value));
            }
            body.push('\n');
        }

        // Prepare labels
        let label_args: Vec<String> = metadata
            .labels
            .iter()
            .map(|l| l.as_str().to_string())
            .collect();

        // Build gh pr create command
        let mut args = vec![
            "pr".to_string(),
            "create".to_string(),
            "--title".to_string(),
            metadata.pr_title().to_string(),
            "--base".to_string(),
            base_branch.to_string(),
            "--body".to_string(),
            body.to_string(),
        ];

        if !label_args.is_empty() {
            args.push("--label".to_string());
            args.push(label_args.join(","));
        }

        // Check if draft is requested via label
        let is_draft = metadata.labels.contains(&PrLabel::StatusDraft);
        if is_draft && self.config.allow_draft {
            args.push("--draft".to_string());
        }

        debug!(args = ?args, "Creating PR with gh");

        let output = tokio::process::Command::new("gh")
            .args(&args)
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| PrCreationError::GitFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(PrCreationError::FailedToCreate(format!(
                "gh failed: {} {}",
                stdout, stderr
            )));
        }

        // Parse PR URL from output
        let stdout = String::from_utf8_lossy(&output.stdout);
        let pr_url = stdout.trim().to_string();

        // Extract PR number from URL
        let pr_number = extract_pr_number_from_url(&pr_url);

        let result = PrResult {
            pr_number,
            pr_url: Some(pr_url),
            created: true,
            is_draft,
            branch_name: current_branch,
            base_branch: base_branch.to_string(),
            labels_applied: label_args,
            error: None,
        };

        // Track the PR
        {
            let mut prs = self.created_prs.write().await;
            prs.insert(result.branch_name.clone(), result.clone());
        }

        info!(
            branch = %result.branch_name,
            pr_url = %result.pr_url.as_ref().unwrap_or(&"<none>".to_string()),
            "PR created successfully"
        );

        Ok(result)
    }

    /// Create a PR using git remote and generate instructions
    /// (fallback when gh CLI is not available)
    pub async fn create_pr_instructions(
        &self,
        metadata: &PrMetadata,
        _evidence: Option<&EvidenceSummary>,
        cwd: &Path,
    ) -> Result<PrResult, PrCreationError> {
        let current_branch = self.get_current_branch(cwd).await?;
        let base_branch = metadata.base_branch(&self.config.default_base_branch);
        let remote_url = self.get_remote_url(cwd).await?;

        // Build the PR body for reference
        let mut body = String::new();

        if let Some(ref desc) = metadata.description {
            body.push_str(desc);
            body.push_str("\n\n");
        }

        if self.config.include_task_metadata {
            body.push_str("## Task Information\n\n");
            if let Some(task_id) = metadata.task_id {
                body.push_str(&format!("- **Task ID**: {}\n", task_id));
            }
            body.push_str(&format!("- **Branch**: {}\n", current_branch));
            body.push('\n');
        }

        if self.config.include_diff_stats {
            match self.get_diff_summary(cwd).await {
                Ok(stats) => {
                    body.push_str("## Changes Summary\n\n");
                    body.push_str("```\n");
                    body.push_str(&stats);
                    body.push_str("```\n\n");
                }
                Err(e) => {
                    warn!(error = %e, "Could not get diff summary");
                }
            }
        }

        if let Some(ev) = _evidence {
            body.push_str(&ev.to_markdown());
            body.push('\n');
        }

        // Generate instructions
        let instructions = format!(
            r#"# PR Creation Instructions

## Remote Repository
{}

## Branch Information
- **Head Branch**: {}
- **Base Branch**: {}

## PR Title
{}

## PR Body
{}

## Labels to Apply
{}

---
*To create the PR, use your git hosting provider's web interface or push the branch and create a PR there.*
"#,
            remote_url,
            current_branch,
            base_branch,
            metadata.pr_title(),
            body,
            metadata
                .labels
                .iter()
                .map(|l| format!("- {}", l.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        );

        info!(
            branch = %current_branch,
            "PR instructions generated (gh CLI not available)"
        );

        // Return a result indicating PR was not actually created
        Ok(PrResult {
            pr_number: None,
            pr_url: None,
            created: false,
            is_draft: false,
            branch_name: current_branch,
            base_branch: base_branch.to_string(),
            labels_applied: metadata.labels.iter().map(|l| l.as_str().to_string()).collect(),
            error: Some(format!(
                "gh CLI not available. Instructions:\n\n{}",
                instructions
            )),
        })
    }

    /// Create a PR with the given metadata and evidence
    pub async fn create_pr(
        &self,
        metadata: PrMetadata,
        evidence: Option<EvidenceSummary>,
        cwd: &Path,
    ) -> Result<PrResult, PrCreationError> {
        // Validate we have something to PR
        if self.config.require_validation && evidence.is_none() {
            return Err(PrCreationError::ValidationRequired);
        }

        // Check if there are commits to PR
        let current_branch = self.get_current_branch(cwd).await?;
        let output = tokio::process::Command::new("git")
            .args(["log", "--oneline", "-1"])
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| PrCreationError::GitFailed(e.to_string()))?;

        if !output.status.success() || String::from_utf8_lossy(&output.stdout).trim().is_empty() {
            return Err(PrCreationError::NoCommits(current_branch));
        }

        // Try gh CLI first, fall back to instructions
        if self.is_gh_available() {
            self.create_pr_with_gh(&metadata, evidence.as_ref(), cwd)
                .await
        } else {
            self.create_pr_instructions(&metadata, evidence.as_ref(), cwd)
                .await
        }
    }

    /// Generate a PR description template
    pub fn generate_pr_template(
        &self,
        metadata: &PrMetadata,
        _evidence: Option<&EvidenceSummary>,
    ) -> String {
        let mut template = String::new();

        // Title suggestion
        template.push_str(&format!(
            "## Suggested Title\n{}\n\n",
            metadata.pr_title()
        ));

        // Description placeholder
        template.push_str("## Description\n\n[Describe the changes and their rationale]\n\n");

        // Task metadata
        if self.config.include_task_metadata {
            template.push_str("## Task Information\n\n");
            if let Some(task_id) = metadata.task_id {
                template.push_str(&format!("- **Task ID**: {}\n", task_id));
            }
            template.push_str("- **Branch**: [branch name]\n");
            template.push('\n');
        }

        // Evidence placeholder
        if self.config.require_validation {
            template.push_str("## Validation Evidence\n\n[Validation results will be appended here]\n\n");
        }

        // Labels suggestion
        if !metadata.labels.is_empty() {
            template.push_str("## Suggested Labels\n\n");
            for label in &metadata.labels {
                template.push_str(&format!("- {}\n", label.as_str()));
            }
            template.push('\n');
        }

        // Checklist
        template.push_str("## Checklist\n\n");
        template.push_str("- [ ] Tests added/updated\n");
        template.push_str("- [ ] Documentation updated\n");
        template.push_str("- [ ] Code follows project conventions\n");
        template.push_str("- [ ] Validation gates passed\n");

        template
    }
}

impl Default for PrCreator {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract PR number from GitHub URL
fn extract_pr_number_from_url(url: &str) -> Option<u32> {
    // URLs like https://github.com/owner/repo/pull/123
    if let Some(idx) = url.rfind("/pull/") {
        let num_str = &url[idx + 6..];
        return num_str.parse().ok();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pr_metadata_new() {
        let meta = PrMetadata::new("Add new feature");
        assert_eq!(meta.task_description, "Add new feature");
        assert!(meta.title.is_none());
        assert!(meta.labels.is_empty());
    }

    #[test]
    fn test_pr_metadata_builder() {
        let meta = PrMetadata::new("Fix bug")
            .with_task_id(Uuid::new_v4())
            .with_title("Critical bug fix")
            .with_description("Fixes the login issue")
            .with_base_branch("develop")
            .with_label(PrLabel::TypeBugfix)
            .with_label(PrLabel::PriorityHigh);

        assert_eq!(meta.pr_title(), "Critical bug fix");
        assert_eq!(meta.base_branch(&"main"), "develop");
        assert_eq!(meta.labels.len(), 2);
    }

    #[test]
    fn test_pr_label_strings() {
        assert_eq!(PrLabel::TypeFeature.as_str(), "type:feature");
        assert_eq!(PrLabel::TypeBugfix.as_str(), "type:bugfix");
        assert_eq!(PrLabel::StatusDraft.as_str(), "status:draft");
        assert_eq!(PrLabel::Milestone("v1".to_string()).as_str(), "milestone:v1");
    }

    #[test]
    fn test_evidence_summary_new() {
        let ev = EvidenceSummary::new();
        assert!(ev.overall_passed);
        assert_eq!(ev.confidence_score, 1.0);
    }

    #[test]
    fn test_evidence_summary_markdown() {
        let mut ev = EvidenceSummary::new();
        ev.tests_passed = 10;
        ev.tests_failed = 2;
        ev.overall_passed = false;

        let md = ev.to_markdown();
        assert!(md.contains("FAILED"));
        assert!(md.contains("10"));
        assert!(md.contains("2"));
    }

    #[test]
    fn test_pr_creator_config_default() {
        let config = PrCreatorConfig::default();
        assert_eq!(config.default_base_branch, "main");
        assert!(config.require_validation);
    }

    #[test]
    fn test_pr_creator_new() {
        let creator = PrCreator::new();
        assert_eq!(creator.config().default_base_branch, "main");
    }

    #[test]
    fn test_extract_pr_number_from_url() {
        assert_eq!(
            extract_pr_number_from_url("https://github.com/owner/repo/pull/123"),
            Some(123)
        );
        assert_eq!(
            extract_pr_number_from_url("https://github.com/owner/repo/pull/4567"),
            Some(4567)
        );
        assert_eq!(extract_pr_number_from_url("not a url"), None);
    }

    #[test]
    fn test_pr_template_generation() {
        let creator = PrCreator::new();
        let metadata = PrMetadata::new("Test PR")
            .with_task_id(Uuid::new_v4())
            .with_label(PrLabel::TypeFeature);

        let template = creator.generate_pr_template(&metadata, None);
        assert!(template.contains("Test PR"));
        assert!(template.contains("Task ID"));
        assert!(template.contains("type:feature"));
    }

    #[tokio::test]
    async fn test_pr_creator_created_count() {
        let creator = PrCreator::new();
        assert_eq!(creator.created_count().await, 0);
    }

    #[test]
    fn test_evidence_summary_with_failed_tests() {
        let ev = EvidenceSummary {
            tests_passed: 5,
            tests_failed: 3,
            tests_skipped: 1,
            lint_passed: true,
            security_passed: false,
            overall_passed: false,
            confidence_score: 0.6,
            evidence_pack_id: Some(Uuid::new_v4()),
            notes: Some("Security issues found".to_string()),
        };

        let md = ev.to_markdown();
        assert!(md.contains("5"));
        assert!(md.contains("3"));
        assert!(md.contains("1"));
        assert!(md.contains("FAILED"));
        assert!(md.contains("Security"));
        assert!(md.contains("60%"));
    }
}
