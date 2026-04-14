//! GitHub Merge Queue and Mergify integration for atomic stacked PR merges.
//!
//! This module provides:
//! - [`MergeQueue`] - manages the merge queue with GitHub/Mergify integration
//! - [`MergeQueueEntry`] - represents a PR in the merge queue
//! - [`MergeProvider`] - abstraction for different merge providers (GitHub, Mergify)
//! - Atomic stacked PR merging support
//!
//! # Architecture
//!
//! ```ignore
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                       MergeQueue                                │
//! │  ┌─────────────────┐  ┌──────────────────┐  ┌────────────────┐  │
//! │  │  add_pr()       │  │  merge_next()    │  │  cancel_pr()   │  │
//! │  │  -> EntryID    │  │  -> MergeResult │  │  -> bool       │  │
//! │  └─────────────────┘  └──────────────────┘  └────────────────┘  │
//! │  ┌─────────────────────────────────────────────────────────────┐  │
//! │  │  provider: Box<dyn MergeProvider>                         │  │
//! │  │  stacked_pr_manager: PrStackManager                       │  │
//! │  │  tiered_merge: TieredMerge                               │  │
//! │  └─────────────────────────────────────────────────────────────┘  │
//! └─────────────────────────────────────────────────────────────────┘
//!
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                    MergeQueueEntry                               │
//! │  ┌─────────────────┐  ┌──────────────────┐  ┌────────────────┐  │
//! │  │  pr_id: String │  │  stack_id: Uuid  │  │  status        │  │
//! │  │  branch: String│  │  priority: u8   │  │  MergeStatus   │  │
//! │  └─────────────────┘  └──────────────────┘  └────────────────┘  │
//! └─────────────────────────────────────────────────────────────────┘
//!
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                     MergeProvider (trait)                       │
//! │  ┌─────────────────┐  ┌──────────────────┐  ┌────────────────┐  │
//! │  │  add_to_queue() │  │  merge_at_head() │  │  remove_from() │  │
//! │  │  -> Result      │  │  -> Result       │  │  -> Result     │  │
//! │  └─────────────────┘  └──────────────────┘  └────────────────┘  │
//! └─────────────────────────────────────────────────────────────────┘
//! ```

use crate::stacked_prs::{PrStackManager, StackedPrConfig};
use crate::tiered_merge::{MergeEligibility, MergeStrategy, TieredMerge};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Errors that can occur during merge queue operations
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum MergeQueueError {
    #[error("Entry '{0}' not found in queue")]
    EntryNotFound(String),

    #[error("Queue is empty, nothing to merge")]
    QueueEmpty,

    #[error("Cannot merge: {0}")]
    CannotMerge(String),

    #[error("Provider error: {0}")]
    ProviderError(String),

    #[error("Stack not found: {0}")]
    StackNotFound(Uuid),

    #[error("Invalid status transition from {from} to {to}")]
    InvalidStatusTransition { from: String, to: String },

    #[error("Atomic merge failed, rollback required")]
    AtomicMergeFailed(String),

    #[error("PR {0} is not mergeable (status: {1})")]
    NotMergeable(String, String),
}

/// Status of a PR in the merge queue
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeStatus {
    /// PR is queued and waiting
    Queued,
    /// PR is at the head of the queue, ready to merge
    AtHead,
    /// PR is being merged (in progress)
    Merging,
    /// PR has been successfully merged
    Merged,
    /// PR was removed from queue (cancelled or superseded)
    Removed,
    /// PR failed to merge
    Failed,
    /// PR is blocked (e.g., CI failing, review pending)
    Blocked,
}

impl MergeStatus {
    /// Check if this status allows the PR to be merged
    pub fn can_merge(&self) -> bool {
        matches!(self, MergeStatus::AtHead | MergeStatus::Blocked)
    }

    /// Check if this is a terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            MergeStatus::Merged | MergeStatus::Removed | MergeStatus::Failed
        )
    }

    /// Get string representation for provider API
    pub fn as_str(&self) -> &'static str {
        match self {
            MergeStatus::Queued => "queued",
            MergeStatus::AtHead => "at_head",
            MergeStatus::Merging => "merging",
            MergeStatus::Merged => "merged",
            MergeStatus::Removed => "removed",
            MergeStatus::Failed => "failed",
            MergeStatus::Blocked => "blocked",
        }
    }
}

/// Result of a merge operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeResult {
    /// Whether the merge was successful
    pub success: bool,
    /// PR identifier that was merged
    pub pr_id: String,
    /// SHA of the merge commit (if successful)
    pub merge_sha: Option<String>,
    /// Message describing the result
    pub message: String,
    /// Time taken for the merge operation
    pub duration_ms: u64,
}

impl MergeResult {
    /// Create a successful merge result
    pub fn success(pr_id: String, merge_sha: String, duration_ms: u64) -> Self {
        Self {
            success: true,
            pr_id,
            merge_sha: Some(merge_sha),
            message: "Merge completed successfully".to_string(),
            duration_ms,
        }
    }

    /// Create a failed merge result
    pub fn failure(pr_id: String, reason: String, duration_ms: u64) -> Self {
        Self {
            success: false,
            pr_id,
            merge_sha: None,
            message: reason,
            duration_ms,
        }
    }
}

/// A PR entry in the merge queue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeQueueEntry {
    /// Unique entry identifier
    pub id: String,
    /// PR number (e.g., "123")
    pub pr_number: String,
    /// Branch name of the PR
    pub branch: String,
    /// Base branch (target)
    pub base_branch: String,
    /// Stack ID this PR belongs to (for stacked PRs)
    pub stack_id: Option<Uuid>,
    /// Current merge status
    pub status: MergeStatus,
    /// Priority in queue (higher = more urgent)
    pub priority: u8,
    /// When the PR was added to queue
    pub queued_at: DateTime<Utc>,
    /// When status last changed
    pub status_changed_at: DateTime<Utc>,
    /// Number of times we've attempted to merge this PR
    pub merge_attempts: u32,
    /// Last error message if merge failed
    pub last_error: Option<String>,
    /// Required checks for this PR (CI, reviews, etc.)
    pub required_checks: Vec<String>,
    /// Whether all required checks have passed
    pub checks_passed: bool,
}

impl MergeQueueEntry {
    /// Create a new merge queue entry
    pub fn new(
        pr_number: String,
        branch: String,
        base_branch: String,
        stack_id: Option<Uuid>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: format!("queue-{}", Uuid::new_v4()),
            pr_number,
            branch,
            base_branch,
            stack_id,
            status: MergeStatus::Queued,
            priority: 50, // Default priority
            queued_at: now,
            status_changed_at: now,
            merge_attempts: 0,
            last_error: None,
            required_checks: Vec::new(),
            checks_passed: false,
        }
    }

    /// Set the status and update timestamp
    pub fn set_status(&mut self, status: MergeStatus) {
        self.status = status;
        self.status_changed_at = Utc::now();
    }

    /// Increment merge attempts and set error
    pub fn record_merge_attempt(&mut self, success: bool, error: Option<String>) {
        self.merge_attempts += 1;
        if !success {
            self.last_error = error;
            self.status = MergeStatus::Failed;
        }
        self.status_changed_at = Utc::now();
    }

    /// Check if this entry can be merged (considering status and checks)
    pub fn can_be_merged(&self) -> bool {
        self.status.can_merge() && self.checks_passed
    }
}

/// Configuration for merge queue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeQueueConfig {
    /// Maximum PRs in the queue (0 = unlimited)
    pub max_queue_size: usize,
    /// Maximum merge attempts before giving up
    pub max_merge_attempts: u32,
    /// Wait time between merge attempts (milliseconds)
    pub retry_delay_ms: u64,
    /// Whether to use atomic stacked merges
    pub atomic_stacked_merges: bool,
    /// Whether to enable Mergify (vs GitHub native merge queue)
    pub use_mergify: bool,
    /// Mergify queue name (if using Mergify)
    pub mergify_queue_name: Option<String>,
    /// GitHub merge queue settings
    pub github_merge_queue: GitHubMergeQueueConfig,
}

impl Default for MergeQueueConfig {
    fn default() -> Self {
        Self {
            max_queue_size: 100,
            max_merge_attempts: 3,
            retry_delay_ms: 5000,
            atomic_stacked_merges: true,
            use_mergify: false,
            mergify_queue_name: None,
            github_merge_queue: GitHubMergeQueueConfig::default(),
        }
    }
}

/// GitHub merge queue configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubMergeQueueConfig {
    /// Use GitHub's merge queue feature
    pub enabled: bool,
    /// Merge method (merge, squash, rebase)
    pub merge_method: GitHubMergeMethod,
    /// Group PRs by branch name pattern
    pub group_by_pattern: Option<String>,
    /// Minimum PRs to start merging
    pub min_group_size: u32,
    /// Maximum PRs to merge per cycle
    pub max_batch_size: u32,
}

impl Default for GitHubMergeQueueConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            merge_method: GitHubMergeMethod::Merge,
            group_by_pattern: None,
            min_group_size: 1,
            max_batch_size: 5,
        }
    }
}

/// GitHub merge method
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitHubMergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl GitHubMergeMethod {
    /// Get string representation for GitHub API
    pub fn as_str(&self) -> &'static str {
        match self {
            GitHubMergeMethod::Merge => "merge",
            GitHubMergeMethod::Squash => "squash",
            GitHubMergeMethod::Rebase => "rebase",
        }
    }
}

/// Trait for merge providers (GitHub, Mergify, etc.)
pub trait MergeProvider: Send + Sync {
    /// Add a PR to the merge queue
    fn add_to_queue(&self, entry: &MergeQueueEntry) -> Result<(), MergeQueueError>;

    /// Remove a PR from the merge queue
    fn remove_from_queue(&self, pr_number: &str) -> Result<(), MergeQueueError>;

    /// Get the status of a PR in the queue
    fn get_queue_status(&self, pr_number: &str) -> Result<MergeStatus, MergeQueueError>;

    /// Trigger merge of PRs at head of queue
    fn merge_at_head(&self, entries: &[MergeQueueEntry]) -> Result<MergeResult, MergeQueueError>;

    /// Update required checks for a PR
    fn update_checks(&self, pr_number: &str, checks: &[String]) -> Result<(), MergeQueueError>;

    /// Get whether all required checks have passed
    fn check_status(&self, pr_number: &str) -> Result<bool, MergeQueueError>;

    /// Get provider name for debugging
    fn provider_name(&self) -> &'static str;
}

/// GitHub merge queue provider (stubbed for MVP)
pub struct GitHubMergeProvider {
    /// Repository identifier (owner/repo)
    #[allow(dead_code)]
    repo: String,
    /// GitHub API token (if provided)
    token: Option<String>,
    /// Configuration
    #[allow(dead_code)]
    config: GitHubMergeQueueConfig,
}

impl GitHubMergeProvider {
    /// Create a new GitHub merge provider
    pub fn new(repo: String, token: Option<String>) -> Self {
        Self {
            repo,
            token,
            config: GitHubMergeQueueConfig::default(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(
        repo: String,
        token: Option<String>,
        config: GitHubMergeQueueConfig,
    ) -> Self {
        Self {
            repo,
            token,
            config,
        }
    }

    /// Check if we have authentication
    fn is_authenticated(&self) -> bool {
        self.token.is_some()
    }
}

impl MergeProvider for GitHubMergeProvider {
    fn add_to_queue(&self, entry: &MergeQueueEntry) -> Result<(), MergeQueueError> {
        info!(
            pr_number = %entry.pr_number,
            branch = %entry.branch,
            provider = "github",
            "Adding PR to merge queue"
        );

        // Stub: In real implementation, this would call GitHub API
        // POST /repos/{owner}/{repo}/pulls/{pull_number}/mergequeue

        if !self.is_authenticated() {
            debug!("GitHub provider not authenticated, queue operation stubbed");
        }

        Ok(())
    }

    fn remove_from_queue(&self, pr_number: &str) -> Result<(), MergeQueueError> {
        info!(
            pr_number = %pr_number,
            provider = "github",
            "Removing PR from merge queue"
        );

        // Stub: Would call DELETE /repos/{owner}/{repo}/mergequeue/...
        Ok(())
    }

    fn get_queue_status(&self, pr_number: &str) -> Result<MergeStatus, MergeQueueError> {
        debug!(
            pr_number = %pr_number,
            provider = "github",
            "Getting queue status"
        );

        // Stub: Would call GET /repos/{owner}/{repo}/mergequeue/...
        // For MVP, just return Queued
        Ok(MergeStatus::Queued)
    }

    fn merge_at_head(&self, entries: &[MergeQueueEntry]) -> Result<MergeResult, MergeQueueError> {
        if entries.is_empty() {
            return Err(MergeQueueError::QueueEmpty);
        }

        let start = std::time::Instant::now();
        let entry = &entries[0];

        info!(
            pr_number = %entry.pr_number,
            branch = %entry.branch,
            provider = "github",
            "Merging PR at head of queue"
        );

        // Stub: Would call POST /repos/{owner}/{repo}/mergequeue/{id}/merge
        // For MVP, simulate merge success

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(MergeResult::success(
            entry.pr_number.clone(),
            format!("sha-{}", Uuid::new_v4()),
            duration_ms,
        ))
    }

    fn update_checks(&self, pr_number: &str, checks: &[String]) -> Result<(), MergeQueueError> {
        debug!(
            pr_number = %pr_number,
            checks = ?checks,
            provider = "github",
            "Updating required checks"
        );

        // Stub: Would update required checks via GitHub API
        Ok(())
    }

    fn check_status(&self, pr_number: &str) -> Result<bool, MergeQueueError> {
        debug!(
            pr_number = %pr_number,
            provider = "github",
            "Checking if PR can be merged"
        );

        // Stub: Would call GitHub API to get CI status and review status
        // For MVP, always return true (checks passed)
        Ok(true)
    }

    fn provider_name(&self) -> &'static str {
        "github"
    }
}

/// Mergify merge queue provider (stubbed for MVP)
pub struct MergifyProvider {
    /// Mergify configuration token
    token: Option<String>,
    /// Platform URL
    #[allow(dead_code)]
    platform_url: String,
    /// Queue name
    queue_name: String,
}

impl MergifyProvider {
    /// Create a new Mergify provider
    pub fn new(token: Option<String>, queue_name: String) -> Self {
        Self {
            token,
            platform_url: "https://api.mergify.com".to_string(),
            queue_name,
        }
    }

    /// Check if we have authentication
    fn is_authenticated(&self) -> bool {
        self.token.is_some()
    }
}

impl MergeProvider for MergifyProvider {
    fn add_to_queue(&self, entry: &MergeQueueEntry) -> Result<(), MergeQueueError> {
        info!(
            pr_number = %entry.pr_number,
            branch = %entry.branch,
            queue = %self.queue_name,
            provider = "mergify",
            "Adding PR to Mergify queue"
        );

        // Stub: Would call Mergify API
        // POST /v1/queues/{queue_name}/pull_requests

        if !self.is_authenticated() {
            debug!("Mergify provider not authenticated, queue operation stubbed");
        }

        Ok(())
    }

    fn remove_from_queue(&self, pr_number: &str) -> Result<(), MergeQueueError> {
        info!(
            pr_number = %pr_number,
            queue = %self.queue_name,
            provider = "mergify",
            "Removing PR from Mergify queue"
        );

        // Stub: Would call DELETE /v1/queues/{queue_name}/pull_requests/...
        Ok(())
    }

    fn get_queue_status(&self, pr_number: &str) -> Result<MergeStatus, MergeQueueError> {
        debug!(
            pr_number = %pr_number,
            queue = %self.queue_name,
            provider = "mergify",
            "Getting queue status"
        );

        // Stub: Would call GET /v1/queues/{queue_name}/pull_requests/...
        Ok(MergeStatus::Queued)
    }

    fn merge_at_head(&self, entries: &[MergeQueueEntry]) -> Result<MergeResult, MergeQueueError> {
        if entries.is_empty() {
            return Err(MergeQueueError::QueueEmpty);
        }

        let start = std::time::Instant::now();
        let entry = &entries[0];

        info!(
            pr_number = %entry.pr_number,
            branch = %entry.branch,
            queue = %self.queue_name,
            provider = "mergify",
            "Triggering Mergify merge for PR at head"
        );

        // Stub: Would call POST /v1/queues/{queue_name}/actions/merge

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(MergeResult::success(
            entry.pr_number.clone(),
            format!("sha-{}", Uuid::new_v4()),
            duration_ms,
        ))
    }

    fn update_checks(&self, pr_number: &str, checks: &[String]) -> Result<(), MergeQueueError> {
        debug!(
            pr_number = %pr_number,
            checks = ?checks,
            provider = "mergify",
            "Updating required checks"
        );

        // Stub: Would update Mergify queue rules
        Ok(())
    }

    fn check_status(&self, pr_number: &str) -> Result<bool, MergeQueueError> {
        debug!(
            pr_number = %pr_number,
            provider = "mergify",
            "Checking if PR can be merged"
        );

        // Stub: Would check Mergify conditions
        Ok(true)
    }

    fn provider_name(&self) -> &'static str {
        "mergify"
    }
}

/// Stub provider for when no real provider is configured
pub struct StubMergeProvider;

impl MergeProvider for StubMergeProvider {
    fn add_to_queue(&self, entry: &MergeQueueEntry) -> Result<(), MergeQueueError> {
        debug!(
            pr_number = %entry.pr_number,
            branch = %entry.branch,
            provider = "stub",
            "Stub: Adding PR to queue"
        );
        Ok(())
    }

    fn remove_from_queue(&self, pr_number: &str) -> Result<(), MergeQueueError> {
        debug!(pr_number = %pr_number, provider = "stub", "Stub: Removing PR from queue");
        Ok(())
    }

    fn get_queue_status(&self, pr_number: &str) -> Result<MergeStatus, MergeQueueError> {
        debug!(
            pr_number = %pr_number,
            provider = "stub",
            "Stub: Getting queue status"
        );
        Ok(MergeStatus::Queued)
    }

    fn merge_at_head(&self, entries: &[MergeQueueEntry]) -> Result<MergeResult, MergeQueueError> {
        if entries.is_empty() {
            return Err(MergeQueueError::QueueEmpty);
        }

        let start = std::time::Instant::now();
        let entry = &entries[0];

        info!(
            pr_number = %entry.pr_number,
            provider = "stub",
            "Stub: Merging PR at head"
        );

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(MergeResult::success(
            entry.pr_number.clone(),
            format!("stub-sha-{}", Uuid::new_v4()),
            duration_ms,
        ))
    }

    fn update_checks(&self, _pr_number: &str, _checks: &[String]) -> Result<(), MergeQueueError> {
        Ok(())
    }

    fn check_status(&self, _pr_number: &str) -> Result<bool, MergeQueueError> {
        Ok(true)
    }

    fn provider_name(&self) -> &'static str {
        "stub"
    }
}

/// The main merge queue manager
pub struct MergeQueue {
    /// Queue entries indexed by PR number
    entries: HashMap<String, MergeQueueEntry>,
    /// Provider for merge operations
    provider: Box<dyn MergeProvider>,
    /// Stacked PR manager for atomic merges
    #[allow(dead_code)]
    stacked_pr_manager: PrStackManager,
    /// Configuration
    config: MergeQueueConfig,
    /// Stats
    stats: MergeQueueStats,
}

impl std::fmt::Debug for MergeQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MergeQueue")
            .field("entries", &self.entries.len())
            .field("config", &self.config)
            .field("stats", &self.stats)
            .finish()
    }
}

/// Statistics for merge queue operations
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MergeQueueStats {
    /// Total PRs merged
    pub total_merged: u64,
    /// Total PRs failed
    pub total_failed: u64,
    /// Total PRs cancelled
    pub total_cancelled: u64,
    /// Average merge time (ms)
    pub avg_merge_time_ms: u64,
    /// Queue size history (for monitoring)
    pub queue_size_history: Vec<u32>,
}

impl MergeQueue {
    /// Create a new merge queue with a stub provider (for testing)
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            provider: Box::new(StubMergeProvider),
            stacked_pr_manager: PrStackManager::new(),
            config: MergeQueueConfig::default(),
            stats: MergeQueueStats::default(),
        }
    }

    /// Create with a GitHub provider
    pub fn with_github(repo: String, token: Option<String>) -> Self {
        Self {
            entries: HashMap::new(),
            provider: Box::new(GitHubMergeProvider::new(repo, token)),
            stacked_pr_manager: PrStackManager::new(),
            config: MergeQueueConfig::default(),
            stats: MergeQueueStats::default(),
        }
    }

    /// Create with a Mergify provider
    pub fn with_mergify(token: Option<String>, queue_name: String) -> Self {
        Self {
            entries: HashMap::new(),
            provider: Box::new(MergifyProvider::new(token, queue_name)),
            stacked_pr_manager: PrStackManager::new(),
            config: MergeQueueConfig::default(),
            stats: MergeQueueStats::default(),
        }
    }

    /// Create with a custom provider
    pub fn with_provider(provider: Box<dyn MergeProvider>) -> Self {
        Self {
            entries: HashMap::new(),
            provider,
            stacked_pr_manager: PrStackManager::new(),
            config: MergeQueueConfig::default(),
            stats: MergeQueueStats::default(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(provider: Box<dyn MergeProvider>, config: MergeQueueConfig) -> Self {
        Self {
            entries: HashMap::new(),
            provider,
            stacked_pr_manager: PrStackManager::new_with_config(StackedPrConfig::default()),
            config,
            stats: MergeQueueStats::default(),
        }
    }

    /// Add a PR to the merge queue
    pub fn add_pr(&mut self, entry: MergeQueueEntry) -> Result<String, MergeQueueError> {
        // Check queue size limit
        if self.config.max_queue_size > 0 && self.entries.len() >= self.config.max_queue_size {
            return Err(MergeQueueError::CannotMerge(format!(
                "Queue is full ({} entries)",
                self.entries.len()
            )));
        }

        let pr_number = entry.pr_number.clone();

        // Add to provider queue
        self.provider.add_to_queue(&entry)?;

        // Add to local queue
        self.entries.insert(pr_number.clone(), entry);

        info!(
            pr_number = %pr_number,
            queue_size = self.entries.len(),
            "PR added to merge queue"
        );

        // Update queue size history
        self.stats
            .queue_size_history
            .push(self.entries.len() as u32);
        if self.stats.queue_size_history.len() > 100 {
            self.stats.queue_size_history.remove(0);
        }

        Ok(pr_number)
    }

    /// Remove a PR from the merge queue
    pub fn remove_pr(&mut self, pr_number: &str) -> Result<MergeQueueEntry, MergeQueueError> {
        // Remove from provider
        self.provider.remove_from_queue(pr_number)?;

        // Remove from local queue
        let entry = self
            .entries
            .remove(pr_number)
            .ok_or(MergeQueueError::EntryNotFound(pr_number.to_string()))?;

        self.stats.total_cancelled += 1;

        info!(pr_number = %pr_number, "PR removed from merge queue");

        Ok(entry)
    }

    /// Get an entry by PR number
    pub fn get_entry(&self, pr_number: &str) -> Option<&MergeQueueEntry> {
        self.entries.get(pr_number)
    }

    /// Get all entries in the queue
    pub fn entries(&self) -> Vec<&MergeQueueEntry> {
        let mut entries: Vec<_> = self.entries.values().collect();
        // Sort by priority (highest first) then by queued time
        entries.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then(a.queued_at.cmp(&b.queued_at))
        });
        entries
    }

    /// Get entries at the head of the queue (ready to merge)
    pub fn get_head_entries(&self) -> Vec<&MergeQueueEntry> {
        let mut head_entries: Vec<_> = self
            .entries
            .values()
            .filter(|e| e.status == MergeStatus::AtHead || e.status == MergeStatus::Blocked)
            .collect();
        head_entries.sort_by(|a, b| a.priority.cmp(&b.priority));
        head_entries
    }

    /// Update entry status
    pub fn update_status(
        &mut self,
        pr_number: &str,
        status: MergeStatus,
    ) -> Result<(), MergeQueueError> {
        let entry = self
            .entries
            .get_mut(pr_number)
            .ok_or(MergeQueueError::EntryNotFound(pr_number.to_string()))?;

        entry.set_status(status);
        Ok(())
    }

    /// Merge the PR at the head of the queue
    pub fn merge_next(&mut self) -> Result<MergeResult, MergeQueueError> {
        // Clone head entries to avoid borrow conflict
        let head_entries: Vec<_> = self.get_head_entries().into_iter().cloned().collect();

        if head_entries.is_empty() {
            return Err(MergeQueueError::QueueEmpty);
        }

        // Check if atomic stacked merges are enabled
        if self.config.atomic_stacked_merges {
            self.merge_stacked_atomic(head_entries)
        } else {
            self.merge_single(&head_entries[0])
        }
    }

    /// Merge a single PR
    fn merge_single(&mut self, entry: &MergeQueueEntry) -> Result<MergeResult, MergeQueueError> {
        let pr_number = entry.pr_number.clone();
        let entry_clone = entry.clone();

        // Update status to merging
        if let Some(e) = self.entries.get_mut(&pr_number) {
            e.set_status(MergeStatus::Merging);
        }

        // Attempt merge via provider
        let result = self.provider.merge_at_head(&[entry_clone]);

        // Update based on result
        if let Some(e) = self.entries.get_mut(&pr_number) {
            match &result {
                Ok(r) if r.success => {
                    e.set_status(MergeStatus::Merged);
                    e.record_merge_attempt(true, None);
                    // First merge: use duration directly; subsequent: average
                    if self.stats.total_merged == 0 {
                        self.stats.avg_merge_time_ms = r.duration_ms;
                    } else {
                        self.stats.avg_merge_time_ms =
                            (self.stats.avg_merge_time_ms + r.duration_ms) / 2;
                    }
                    self.stats.total_merged += 1;
                }
                Ok(r) => {
                    e.set_status(MergeStatus::Failed);
                    e.record_merge_attempt(false, Some(r.message.clone()));
                    self.stats.total_failed += 1;
                }
                Err(err) => {
                    e.set_status(MergeStatus::Failed);
                    e.record_merge_attempt(false, Some(err.to_string()));
                    self.stats.total_failed += 1;
                }
            }
        }

        result
    }

    /// Merge multiple stacked PRs atomically
    fn merge_stacked_atomic(
        &mut self,
        head_entries: Vec<MergeQueueEntry>,
    ) -> Result<MergeResult, MergeQueueError> {
        // Find entries that belong to the same stack
        let stack_id = head_entries.first().and_then(|e| e.stack_id);

        if let Some(stack_id) = stack_id {
            // Get all PRs in this stack that are queued or at head
            let stack_entries: Vec<_> = self
                .entries
                .values()
                .filter(|e| e.stack_id == Some(stack_id) && !e.status.is_terminal())
                .cloned()
                .collect();

            info!(
                stack_id = %stack_id,
                pr_count = stack_entries.len(),
                "Performing atomic stacked merge"
            );

            // Merge all PRs in the stack in order
            let _base_branch = stack_entries
                .first()
                .map(|e| e.base_branch.clone())
                .unwrap_or_default();

            for (i, entry) in stack_entries.iter().enumerate() {
                if i > 0 {
                    // Wait for previous PR to be fully merged before starting next
                    // In a real implementation, we'd check the merge status
                    debug!(
                        pr_number = %entry.pr_number,
                        position = i + 1,
                        "Waiting for previous PR to complete before merging"
                    );
                }

                let result = self.provider.merge_at_head(std::slice::from_ref(entry));

                if let Err(e) = result {
                    // Atomic merge failed - in real implementation, we'd need rollback
                    warn!(
                        pr_number = %entry.pr_number,
                        error = %e,
                        "Atomic stacked merge failed"
                    );
                    return Err(MergeQueueError::AtomicMergeFailed(e.to_string()));
                }

                // Update status for this PR
                if let Some(e) = self.entries.get_mut(&entry.pr_number) {
                    e.set_status(MergeStatus::Merged);
                    e.record_merge_attempt(true, None);
                }

                self.stats.total_merged += 1;
            }

            // Return success for the last PR (overall stack merge completed)
            let last_entry = stack_entries.last().unwrap();
            Ok(MergeResult::success(
                last_entry.pr_number.clone(),
                format!("atomic-stack-{}", stack_id),
                0, // Duration would be accumulated in real implementation
            ))
        } else {
            // No stack, just merge the single head entry
            self.merge_single(&head_entries[0])
        }
    }

    /// Cancel a PR in the queue
    pub fn cancel_pr(&mut self, pr_number: &str) -> Result<(), MergeQueueError> {
        let _entry = self.remove_pr(pr_number)?;
        info!(pr_number = %pr_number, "PR cancelled from merge queue");
        Ok(())
    }

    /// Check and update merge eligibility for all entries
    pub fn refresh_status(&mut self) -> Result<(), MergeQueueError> {
        for entry in self.entries.values_mut() {
            if entry.status.is_terminal() {
                continue;
            }

            // Get status from provider
            match self.provider.get_queue_status(&entry.pr_number) {
                Ok(status) => {
                    if status != entry.status {
                        entry.set_status(status);
                    }
                }
                Err(e) => {
                    warn!(
                        pr_number = %entry.pr_number,
                        error = %e,
                        "Failed to get status from provider"
                    );
                }
            }

            // Check if checks have passed
            match self.provider.check_status(&entry.pr_number) {
                Ok(passed) => {
                    entry.checks_passed = passed;
                }
                Err(e) => {
                    debug!(
                        pr_number = %entry.pr_number,
                        error = %e,
                        "Failed to check status"
                    );
                }
            }
        }

        Ok(())
    }

    /// Get queue statistics
    pub fn stats(&self) -> &MergeQueueStats {
        &self.stats
    }

    /// Get current queue size
    pub fn queue_size(&self) -> usize {
        self.entries.len()
    }

    /// Check if queue is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Evaluate merge eligibility using tiered merge strategy
    pub fn evaluate_merge_eligibility(
        &self,
        plan_risk: swell_core::RiskLevel,
        confidence: f64,
    ) -> MergeEligibility {
        TieredMerge::evaluate(plan_risk, confidence)
    }

    /// Check if a PR is mergeable based on strategy
    pub fn is_mergeable(&self, entry: &MergeQueueEntry, strategy: MergeStrategy) -> bool {
        match strategy {
            MergeStrategy::AutoMerge => entry.can_be_merged(),
            MergeStrategy::AutoMergeWithAiReview => entry.can_be_merged(), // Would also check AI review
            MergeStrategy::HumanReview => false, // Never auto-merge for human review
        }
    }

    /// Get entries that can be merged with given strategy
    pub fn get_mergeable_entries(&self, strategy: MergeStrategy) -> Vec<&MergeQueueEntry> {
        self.entries
            .values()
            .filter(|e| self.is_mergeable(e, strategy))
            .collect()
    }

    /// Sync with external provider (e.g., fetch current queue state from GitHub)
    pub async fn sync_with_provider(&mut self) -> Result<(), MergeQueueError> {
        info!(provider = %self.provider.provider_name(), "Syncing merge queue with provider");

        // In real implementation, this would:
        // 1. Fetch current queue state from provider
        // 2. Update local entries to match
        // 3. Remove entries that no longer exist in provider queue

        Ok(())
    }
}

impl Default for MergeQueue {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- MergeQueueEntry Tests ---

    #[test]
    fn test_merge_queue_entry_creation() {
        let entry = MergeQueueEntry::new(
            "123".to_string(),
            "feat/my-branch".to_string(),
            "main".to_string(),
            Some(Uuid::new_v4()),
        );

        assert_eq!(entry.pr_number, "123");
        assert_eq!(entry.branch, "feat/my-branch");
        assert_eq!(entry.base_branch, "main");
        assert_eq!(entry.status, MergeStatus::Queued);
        assert_eq!(entry.priority, 50);
        assert_eq!(entry.merge_attempts, 0);
        assert!(entry.last_error.is_none());
    }

    #[test]
    fn test_merge_queue_entry_set_status() {
        let mut entry = MergeQueueEntry::new(
            "123".to_string(),
            "feat/my-branch".to_string(),
            "main".to_string(),
            None,
        );

        let original_time = entry.status_changed_at;
        std::thread::sleep(std::time::Duration::from_millis(10));

        entry.set_status(MergeStatus::AtHead);

        assert_eq!(entry.status, MergeStatus::AtHead);
        assert!(entry.status_changed_at > original_time);
    }

    #[test]
    fn test_merge_queue_entry_record_merge_attempt_success() {
        let mut entry = MergeQueueEntry::new(
            "123".to_string(),
            "branch".to_string(),
            "main".to_string(),
            None,
        );

        entry.record_merge_attempt(true, None);

        assert_eq!(entry.merge_attempts, 1);
        assert!(entry.last_error.is_none());
    }

    #[test]
    fn test_merge_queue_entry_record_merge_attempt_failure() {
        let mut entry = MergeQueueEntry::new(
            "123".to_string(),
            "branch".to_string(),
            "main".to_string(),
            None,
        );

        entry.record_merge_attempt(false, Some("CI failed".to_string()));

        assert_eq!(entry.merge_attempts, 1);
        assert_eq!(entry.last_error, Some("CI failed".to_string()));
        assert_eq!(entry.status, MergeStatus::Failed);
    }

    #[test]
    fn test_merge_queue_entry_can_be_merged() {
        let mut entry = MergeQueueEntry::new(
            "123".to_string(),
            "branch".to_string(),
            "main".to_string(),
            None,
        );

        // Queued and checks not passed - cannot merge
        assert!(!entry.can_be_merged());

        entry.set_status(MergeStatus::AtHead);

        // AtHead but checks not passed - cannot merge
        assert!(!entry.can_be_merged());

        entry.checks_passed = true;

        // AtHead and checks passed - can merge
        assert!(entry.can_be_merged());
    }

    // --- MergeStatus Tests ---

    #[test]
    fn test_merge_status_can_merge() {
        assert!(MergeStatus::AtHead.can_merge());
        assert!(MergeStatus::Blocked.can_merge());
        assert!(!MergeStatus::Queued.can_merge());
        assert!(!MergeStatus::Merging.can_merge());
        assert!(!MergeStatus::Merged.can_merge());
    }

    #[test]
    fn test_merge_status_is_terminal() {
        assert!(MergeStatus::Merged.is_terminal());
        assert!(MergeStatus::Removed.is_terminal());
        assert!(MergeStatus::Failed.is_terminal());
        assert!(!MergeStatus::Queued.is_terminal());
        assert!(!MergeStatus::AtHead.is_terminal());
        assert!(!MergeStatus::Merging.is_terminal());
        assert!(!MergeStatus::Blocked.is_terminal());
    }

    #[test]
    fn test_merge_status_as_str() {
        assert_eq!(MergeStatus::Queued.as_str(), "queued");
        assert_eq!(MergeStatus::AtHead.as_str(), "at_head");
        assert_eq!(MergeStatus::Merging.as_str(), "merging");
        assert_eq!(MergeStatus::Merged.as_str(), "merged");
        assert_eq!(MergeStatus::Removed.as_str(), "removed");
        assert_eq!(MergeStatus::Failed.as_str(), "failed");
        assert_eq!(MergeStatus::Blocked.as_str(), "blocked");
    }

    // --- MergeResult Tests ---

    #[test]
    fn test_merge_result_success() {
        let result = MergeResult::success("123".to_string(), "sha-abc".to_string(), 150);

        assert!(result.success);
        assert_eq!(result.pr_id, "123");
        assert_eq!(result.merge_sha, Some("sha-abc".to_string()));
        assert_eq!(result.duration_ms, 150);
    }

    #[test]
    fn test_merge_result_failure() {
        let result = MergeResult::failure("123".to_string(), "CI failed".to_string(), 50);

        assert!(!result.success);
        assert_eq!(result.pr_id, "123");
        assert!(result.merge_sha.is_none());
        assert_eq!(result.message, "CI failed");
        assert_eq!(result.duration_ms, 50);
    }

    // --- MergeQueue Tests ---

    #[test]
    fn test_merge_queue_new() {
        let queue = MergeQueue::new();

        assert!(queue.is_empty());
        assert_eq!(queue.queue_size(), 0);
    }

    #[test]
    fn test_merge_queue_add_pr() {
        let mut queue = MergeQueue::new();

        let entry = MergeQueueEntry::new(
            "123".to_string(),
            "feat/my-branch".to_string(),
            "main".to_string(),
            None,
        );

        let result = queue.add_pr(entry);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "123");
        assert_eq!(queue.queue_size(), 1);
    }

    #[test]
    fn test_merge_queue_add_pr_max_size() {
        let mut queue = MergeQueue::with_config(
            Box::new(StubMergeProvider),
            MergeQueueConfig {
                max_queue_size: 2,
                ..Default::default()
            },
        );

        let entry1 = MergeQueueEntry::new(
            "1".to_string(),
            "branch1".to_string(),
            "main".to_string(),
            None,
        );
        let entry2 = MergeQueueEntry::new(
            "2".to_string(),
            "branch2".to_string(),
            "main".to_string(),
            None,
        );
        let entry3 = MergeQueueEntry::new(
            "3".to_string(),
            "branch3".to_string(),
            "main".to_string(),
            None,
        );

        assert!(queue.add_pr(entry1).is_ok());
        assert!(queue.add_pr(entry2).is_ok());
        assert!(queue.add_pr(entry3).is_err()); // Queue full
    }

    #[test]
    fn test_merge_queue_remove_pr() {
        let mut queue = MergeQueue::new();

        let entry = MergeQueueEntry::new(
            "123".to_string(),
            "branch".to_string(),
            "main".to_string(),
            None,
        );
        queue.add_pr(entry).unwrap();

        let removed = queue.remove_pr("123");

        assert!(removed.is_ok());
        assert!(queue.is_empty());
    }

    #[test]
    fn test_merge_queue_remove_nonexistent() {
        let mut queue = MergeQueue::new();

        let result = queue.remove_pr("nonexistent");

        assert!(result.is_err());
    }

    #[test]
    fn test_merge_queue_get_entry() {
        let mut queue = MergeQueue::new();

        let entry = MergeQueueEntry::new(
            "123".to_string(),
            "branch".to_string(),
            "main".to_string(),
            None,
        );
        queue.add_pr(entry).unwrap();

        let retrieved = queue.get_entry("123");

        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().pr_number, "123");
    }

    #[test]
    fn test_merge_queue_get_entry_nonexistent() {
        let queue = MergeQueue::new();

        assert!(queue.get_entry("nonexistent").is_none());
    }

    #[test]
    fn test_merge_queue_entries_sorted_by_priority() {
        let mut queue = MergeQueue::new();

        let mut entry1 = MergeQueueEntry::new(
            "1".to_string(),
            "branch1".to_string(),
            "main".to_string(),
            None,
        );
        entry1.priority = 10;

        let mut entry2 = MergeQueueEntry::new(
            "2".to_string(),
            "branch2".to_string(),
            "main".to_string(),
            None,
        );
        entry2.priority = 100;

        let mut entry3 = MergeQueueEntry::new(
            "3".to_string(),
            "branch3".to_string(),
            "main".to_string(),
            None,
        );
        entry3.priority = 50;

        queue.add_pr(entry1).unwrap();
        queue.add_pr(entry2).unwrap();
        queue.add_pr(entry3).unwrap();

        let entries = queue.entries();

        // Should be sorted by priority (highest first): 2, 3, 1
        assert_eq!(entries[0].pr_number, "2");
        assert_eq!(entries[1].pr_number, "3");
        assert_eq!(entries[2].pr_number, "1");
    }

    #[test]
    fn test_merge_queue_get_head_entries() {
        let mut queue = MergeQueue::new();

        let mut entry1 = MergeQueueEntry::new(
            "1".to_string(),
            "branch1".to_string(),
            "main".to_string(),
            None,
        );
        entry1.set_status(MergeStatus::Queued);

        let mut entry2 = MergeQueueEntry::new(
            "2".to_string(),
            "branch2".to_string(),
            "main".to_string(),
            None,
        );
        entry2.set_status(MergeStatus::AtHead);

        let mut entry3 = MergeQueueEntry::new(
            "3".to_string(),
            "branch3".to_string(),
            "main".to_string(),
            None,
        );
        entry3.set_status(MergeStatus::Merged);

        queue.add_pr(entry1).unwrap();
        queue.add_pr(entry2).unwrap();
        queue.add_pr(entry3).unwrap();

        let head = queue.get_head_entries();

        assert_eq!(head.len(), 1);
        assert_eq!(head[0].pr_number, "2");
    }

    #[test]
    fn test_merge_queue_merge_next() {
        let mut queue = MergeQueue::new();

        let mut entry = MergeQueueEntry::new(
            "123".to_string(),
            "branch".to_string(),
            "main".to_string(),
            None,
        );
        entry.set_status(MergeStatus::AtHead);
        entry.checks_passed = true;

        queue.add_pr(entry).unwrap();

        let result = queue.merge_next();

        assert!(result.is_ok());
        assert!(result.unwrap().success);

        // Check entry was updated
        let updated = queue.get_entry("123").unwrap();
        assert_eq!(updated.status, MergeStatus::Merged);
    }

    #[test]
    fn test_merge_queue_merge_next_empty() {
        let mut queue = MergeQueue::new();

        let result = queue.merge_next();

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), MergeQueueError::QueueEmpty));
    }

    #[test]
    fn test_merge_queue_update_status() {
        let mut queue = MergeQueue::new();

        let entry = MergeQueueEntry::new(
            "123".to_string(),
            "branch".to_string(),
            "main".to_string(),
            None,
        );
        queue.add_pr(entry).unwrap();

        queue.update_status("123", MergeStatus::AtHead).unwrap();

        let updated = queue.get_entry("123").unwrap();
        assert_eq!(updated.status, MergeStatus::AtHead);
    }

    #[test]
    fn test_merge_queue_update_status_nonexistent() {
        let mut queue = MergeQueue::new();

        let result = queue.update_status("nonexistent", MergeStatus::AtHead);

        assert!(result.is_err());
    }

    #[test]
    fn test_merge_queue_cancel_pr() {
        let mut queue = MergeQueue::new();

        let entry = MergeQueueEntry::new(
            "123".to_string(),
            "branch".to_string(),
            "main".to_string(),
            None,
        );
        queue.add_pr(entry).unwrap();

        queue.cancel_pr("123").unwrap();

        assert!(queue.is_empty());
    }

    #[test]
    fn test_merge_queue_refresh_status() {
        let mut queue = MergeQueue::new();

        let entry = MergeQueueEntry::new(
            "123".to_string(),
            "branch".to_string(),
            "main".to_string(),
            None,
        );
        queue.add_pr(entry).unwrap();

        let result = queue.refresh_status();

        assert!(result.is_ok());
    }

    #[test]
    fn test_merge_queue_stats() {
        let mut queue = MergeQueue::new();

        let entry = MergeQueueEntry::new(
            "123".to_string(),
            "branch".to_string(),
            "main".to_string(),
            None,
        );
        queue.add_pr(entry).unwrap();

        // Trigger a merge
        if let Some(e) = queue.entries.get_mut("123") {
            e.set_status(MergeStatus::AtHead);
            e.checks_passed = true;
        }

        let result = queue.merge_next().unwrap();
        assert!(result.success);

        let stats = queue.stats();

        assert_eq!(stats.total_merged, 1);
        assert_eq!(stats.total_failed, 0);
    }

    // --- MergeQueueConfig Tests ---

    #[test]
    fn test_merge_queue_config_default() {
        let config = MergeQueueConfig::default();

        assert_eq!(config.max_queue_size, 100);
        assert_eq!(config.max_merge_attempts, 3);
        assert!(config.atomic_stacked_merges);
        assert!(!config.use_mergify);
    }

    // --- GitHubMergeQueueConfig Tests ---

    #[test]
    fn test_github_merge_queue_config_default() {
        let config = GitHubMergeQueueConfig::default();

        assert!(config.enabled);
        assert_eq!(config.merge_method, GitHubMergeMethod::Merge);
        assert_eq!(config.min_group_size, 1);
        assert_eq!(config.max_batch_size, 5);
    }

    // --- GitHubMergeMethod Tests ---

    #[test]
    fn test_github_merge_method_as_str() {
        assert_eq!(GitHubMergeMethod::Merge.as_str(), "merge");
        assert_eq!(GitHubMergeMethod::Squash.as_str(), "squash");
        assert_eq!(GitHubMergeMethod::Rebase.as_str(), "rebase");
    }

    // --- MergeQueue with GitHub Provider Tests ---

    #[test]
    fn test_merge_queue_with_github_provider() {
        let queue = MergeQueue::with_github("owner/repo".to_string(), None);

        assert!(queue.is_empty());
    }

    #[test]
    fn test_merge_queue_github_provider_add_pr() {
        let mut queue = MergeQueue::with_github("owner/repo".to_string(), None);

        let entry = MergeQueueEntry::new(
            "123".to_string(),
            "branch".to_string(),
            "main".to_string(),
            None,
        );

        let result = queue.add_pr(entry);

        assert!(result.is_ok());
    }

    // --- MergeQueue with Mergify Provider Tests ---

    #[test]
    fn test_merge_queue_with_mergify_provider() {
        let queue = MergeQueue::with_mergify(None, "default".to_string());

        assert!(queue.is_empty());
    }

    #[test]
    fn test_merge_queue_mergify_provider_add_pr() {
        let mut queue = MergeQueue::with_mergify(None, "default".to_string());

        let entry = MergeQueueEntry::new(
            "123".to_string(),
            "branch".to_string(),
            "main".to_string(),
            None,
        );

        let result = queue.add_pr(entry);

        assert!(result.is_ok());
    }

    // --- MergeQueue evaluate_merge_eligibility Tests ---

    #[test]
    fn test_merge_queue_evaluate_low_risk_high_confidence() {
        let queue = MergeQueue::new();

        let eligibility = queue.evaluate_merge_eligibility(swell_core::RiskLevel::Low, 0.9);

        assert!(eligibility.can_merge);
        assert_eq!(eligibility.strategy, MergeStrategy::AutoMerge);
    }

    #[test]
    fn test_merge_queue_evaluate_high_risk() {
        let queue = MergeQueue::new();

        let eligibility = queue.evaluate_merge_eligibility(swell_core::RiskLevel::High, 0.9);

        assert!(eligibility.can_merge);
        assert_eq!(eligibility.strategy, MergeStrategy::HumanReview);
    }

    // --- MergeQueue is_mergeable Tests ---

    #[test]
    fn test_merge_queue_is_mergeable_auto_merge() {
        let mut queue = MergeQueue::new();

        let mut entry = MergeQueueEntry::new(
            "123".to_string(),
            "branch".to_string(),
            "main".to_string(),
            None,
        );
        entry.set_status(MergeStatus::AtHead);
        entry.checks_passed = true;
        queue.add_pr(entry).unwrap();

        assert!(queue.is_mergeable(queue.get_entry("123").unwrap(), MergeStrategy::AutoMerge));
    }

    #[test]
    fn test_merge_queue_is_mergeable_human_review() {
        let mut queue = MergeQueue::new();

        let mut entry = MergeQueueEntry::new(
            "123".to_string(),
            "branch".to_string(),
            "main".to_string(),
            None,
        );
        entry.set_status(MergeStatus::AtHead);
        entry.checks_passed = true;
        queue.add_pr(entry).unwrap();

        // Human review strategy should never allow auto-merge
        assert!(!queue.is_mergeable(queue.get_entry("123").unwrap(), MergeStrategy::HumanReview));
    }

    #[test]
    fn test_merge_queue_get_mergeable_entries() {
        let mut queue = MergeQueue::new();

        // Entry that can be merged
        let mut entry1 = MergeQueueEntry::new(
            "1".to_string(),
            "branch1".to_string(),
            "main".to_string(),
            None,
        );
        entry1.set_status(MergeStatus::AtHead);
        entry1.checks_passed = true;

        // Entry that cannot be merged
        let mut entry2 = MergeQueueEntry::new(
            "2".to_string(),
            "branch2".to_string(),
            "main".to_string(),
            None,
        );
        entry2.set_status(MergeStatus::Queued);
        entry2.checks_passed = false;

        queue.add_pr(entry1).unwrap();
        queue.add_pr(entry2).unwrap();

        let mergeable = queue.get_mergeable_entries(MergeStrategy::AutoMerge);

        assert_eq!(mergeable.len(), 1);
        assert_eq!(mergeable[0].pr_number, "1");
    }

    // --- MergeQueue Stats Tests ---

    #[test]
    fn test_merge_queue_stats_after_merge() {
        let mut queue = MergeQueue::new();

        let mut entry = MergeQueueEntry::new(
            "123".to_string(),
            "branch".to_string(),
            "main".to_string(),
            None,
        );
        entry.set_status(MergeStatus::AtHead);
        entry.checks_passed = true;
        queue.add_pr(entry).unwrap();

        let _merge_result = queue.merge_next().unwrap();

        let stats = queue.stats();

        assert_eq!(stats.total_merged, 1);
        // avg_merge_time_ms is tracked but may be 0 in unit tests (instantaneous mock merge)
    }

    // --- Error Messages Tests ---

    #[test]
    fn test_merge_queue_error_entry_not_found() {
        let err = MergeQueueError::EntryNotFound("123".to_string());
        assert_eq!(err.to_string(), "Entry '123' not found in queue");
    }

    #[test]
    fn test_merge_queue_error_queue_empty() {
        let err = MergeQueueError::QueueEmpty;
        assert_eq!(err.to_string(), "Queue is empty, nothing to merge");
    }

    #[test]
    fn test_merge_queue_error_cannot_merge() {
        let err = MergeQueueError::CannotMerge("Queue is full".to_string());
        assert_eq!(err.to_string(), "Cannot merge: Queue is full");
    }

    #[test]
    fn test_merge_queue_error_not_mergeable() {
        let err = MergeQueueError::NotMergeable("123".to_string(), "Queued".to_string());
        assert_eq!(err.to_string(), "PR 123 is not mergeable (status: Queued)");
    }
}
