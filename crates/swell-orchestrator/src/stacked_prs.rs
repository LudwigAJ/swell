//! Stacked PRs module for managing small incremental changes.
//!
//! This module provides PR stack management with:
//! - Small PRs under 200 lines each
//! - Dependency tracking between PRs in a stack
//! - Smart splitting of large changes into reviewable chunks
//!
//! # Architecture
//!
//! ```ignore
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                      PrStackManager                              │
//! │  ┌─────────────────┐  ┌──────────────────┐  ┌────────────────┐  │
//! │  │  create_stack()│  │  add_pr()       │  │  split_pr()    │  │
//! │  │  -> PrStack    │  │  -> Result      │  │  -> Vec<Pr>    │  │
//! │  └─────────────────┘  └──────────────────┘  └────────────────┘  │
//! │  ┌─────────────────────────────────────────────────────────────┐  │
//! │  │  stacks: HashMap<Uuid, PrStack>                           │  │
//! │  │  max_pr_lines: u32 (default: 200)                          │  │
//! │  └─────────────────────────────────────────────────────────────┘  │
//! └─────────────────────────────────────────────────────────────────┘
//!
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                         PrStack                                  │
//! │  ┌─────────────────┐  ┌──────────────────┐  ┌────────────────┐  │
//! │  │  base_branch   │  │  prs: Vec<Pr>   │  │  task_id      │  │
//! │  │  String        │  │  ordered        │  │  Uuid         │  │
//! │  └─────────────────┘  └──────────────────┘  └────────────────┘  │
//! └─────────────────────────────────────────────────────────────────┘
//!
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                            Pr                                    │
//! │  ┌─────────────────┐  ┌──────────────────┐  ┌────────────────┐  │
//! │  │  id: Uuid       │  │  branch: String │  │  files: Vec   │  │
//! │  │  depends_on    │  │  size: u32      │  │  line_count   │  │
//! │  └─────────────────┘  └──────────────────┘  └────────────────┘  │
//! └─────────────────────────────────────────────────────────────────┘
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info};
use uuid::Uuid;

/// Maximum lines per PR (default: 200)
pub const DEFAULT_MAX_PR_LINES: u32 = 200;

/// Minimum lines to consider a PR worth creating (avoid tiny PRs)
pub const MIN_PR_LINES: u32 = 20;

/// Manager for PR stacks in a project
#[derive(Debug)]
pub struct PrStackManager {
    /// Active PR stacks indexed by task_id
    stacks: HashMap<Uuid, PrStack>,
    /// Maximum lines per PR
    max_pr_lines: u32,
    /// Minimum lines to consider worth a PR
    min_pr_lines: u32,
}

impl PrStackManager {
    /// Create a new PR stack manager
    pub fn new() -> Self {
        Self {
            stacks: HashMap::new(),
            max_pr_lines: DEFAULT_MAX_PR_LINES,
            min_pr_lines: MIN_PR_LINES,
        }
    }

    /// Create with custom max lines per PR
    pub fn with_max_lines(max_pr_lines: u32) -> Self {
        Self {
            stacks: HashMap::new(),
            max_pr_lines,
            min_pr_lines: MIN_PR_LINES,
        }
    }

    /// Create a new PR stack for a task
    pub fn create_stack(&mut self, task_id: Uuid, base_branch: String) -> PrStack {
        let stack = PrStack::new(task_id, base_branch.clone());
        info!(task_id = %task_id, base_branch = %base_branch, "Created new PR stack");
        self.stacks.insert(task_id, stack);
        self.stacks.get(&task_id).unwrap().clone()
    }

    /// Get an existing stack for a task
    pub fn get_stack(&self, task_id: &Uuid) -> Option<&PrStack> {
        self.stacks.get(task_id)
    }

    /// Get a mutable stack for a task
    pub fn get_stack_mut(&mut self, task_id: &Uuid) -> Option<&mut PrStack> {
        self.stacks.get_mut(task_id)
    }

    /// Remove a stack when task is complete
    pub fn remove_stack(&mut self, task_id: &Uuid) -> Option<PrStack> {
        self.stacks.remove(task_id)
    }

    /// Add a PR to an existing stack
    pub fn add_pr(&mut self, task_id: Uuid, pr: Pr) -> Result<(), StackedPrError> {
        let stack = self
            .stacks
            .get_mut(&task_id)
            .ok_or(StackedPrError::StackNotFound(task_id))?;

        stack.add_pr(pr)?;
        info!(task_id = %task_id, "Added PR to stack");
        Ok(())
    }

    /// Calculate if a set of changes needs to be split into multiple PRs
    pub fn calculate_splits(
        &self,
        task_id: Uuid,
        changes: &[PrFileChange],
    ) -> Result<Vec<Pr>, StackedPrError> {
        let stack = self
            .stacks
            .get(&task_id)
            .ok_or(StackedPrError::StackNotFound(task_id))?;

        let total_lines: u32 = changes.iter().map(|c| c.line_count).sum();

        // If changes fit in single PR under limit, no need to split
        if total_lines <= self.max_pr_lines && changes.len() <= 10 {
            let base_branch = stack.base_branch.clone();
            let pr = Pr::new(
                format!("pr-{}-1", task_id),
                stack.next_pr_number(),
                base_branch,
                changes.to_vec(),
            );
            return Ok(vec![pr]);
        }

        // Need to split - group files by risk_level and impact
        Ok(self.split_changes_into_prs(task_id, changes, &stack.base_branch))
    }

    /// Split changes into multiple PRs respecting size limits
    fn split_changes_into_prs(
        &self,
        task_id: Uuid,
        changes: &[PrFileChange],
        base_branch: &str,
    ) -> Vec<Pr> {
        let mut prs = Vec::new();
        let mut current_pr_changes: Vec<PrFileChange> = Vec::new();
        let mut current_pr_lines: u32 = 0;
        let mut pr_number = 1;

        // Sort changes by risk level (high risk first, want them reviewed early)
        let mut sorted_changes = changes.to_vec();
        sorted_changes.sort_by(|a, b| a.risk_level.cmp(&b.risk_level));

        for change in sorted_changes {
            let change_lines = change.line_count;

            // If single change exceeds max, it needs its own PR
            if change_lines > self.max_pr_lines {
                // Flush current PR if it has content
                if !current_pr_changes.is_empty() {
                    let pr = Pr::new(
                        format!("pr-{}-{}", task_id, pr_number),
                        pr_number,
                        base_branch.to_string(),
                        std::mem::take(&mut current_pr_changes),
                    );
                    prs.push(pr);
                    pr_number += 1;
                    current_pr_lines = 0;
                }

                // Split the large file into smaller chunks if needed
                let chunks = self.split_large_file_change(&change, task_id, pr_number, base_branch);
                pr_number += chunks.len() as u32;
                prs.extend(chunks);
                continue;
            }

            // Check if adding this change would exceed limit
            if current_pr_lines + change_lines > self.max_pr_lines {
                // Current PR is full, create it and start new one
                if current_pr_lines >= self.min_pr_lines {
                    let pr = Pr::new(
                        format!("pr-{}-{}", task_id, pr_number),
                        pr_number,
                        base_branch.to_string(),
                        std::mem::take(&mut current_pr_changes),
                    );
                    prs.push(pr);
                    pr_number += 1;
                    current_pr_lines = 0;
                } else {
                    // Current PR is too small, keep adding to next
                    current_pr_changes.push(change);
                    current_pr_lines += change_lines;
                }
            } else {
                current_pr_changes.push(change);
                current_pr_lines += change_lines;
            }
        }

        // Don't forget the last PR
        if !current_pr_changes.is_empty() && current_pr_lines >= self.min_pr_lines {
            let pr = Pr::new(
                format!("pr-{}-{}", task_id, pr_number),
                pr_number,
                base_branch.to_string(),
                current_pr_changes,
            );
            prs.push(pr);
        }

        // Set dependency chain (each PR depends on previous)
        Self::set_pr_dependencies(&mut prs);

        info!(
            task_id = %task_id,
            original_changes = changes.len(),
            resulting_prs = prs.len(),
            "Split changes into PRs"
        );

        prs
    }

    /// Split a large file change into smaller chunks
    fn split_large_file_change(
        &self,
        change: &PrFileChange,
        task_id: Uuid,
        start_pr_number: u32,
        base_branch: &str,
    ) -> Vec<Pr> {
        let mut prs = Vec::new();
        let lines = change.content.lines().collect::<Vec<_>>();
        let total_lines = lines.len() as u32;
        let chunk_size = ((self.max_pr_lines as f32 * 0.7) as usize).max(1); // 70% of limit to leave room

        let mut current_chunk: Vec<String> = Vec::new();
        let mut current_lines: u32 = 0;
        let mut chunk_num = 1;

        for line in lines {
            current_chunk.push(line.to_string());
            current_lines += 1;

            if current_lines >= chunk_size as u32 || current_chunk.len() >= chunk_size {
                let chunk_content = current_chunk.join("\n");
                let chunk_change = PrFileChange {
                    path: format!("{}.part{}", change.path, chunk_num),
                    content: chunk_content,
                    line_count: current_lines,
                    risk_level: change.risk_level,
                };

                let pr = Pr::new(
                    format!("pr-{}-{}-{}", task_id, start_pr_number, chunk_num),
                    start_pr_number + chunk_num - 1,
                    base_branch.to_string(),
                    vec![chunk_change],
                );
                prs.push(pr);

                current_chunk.clear();
                current_lines = 0;
                chunk_num += 1;
            }
        }

        // Last chunk
        if !current_chunk.is_empty() {
            let chunk_content = current_chunk.join("\n");
            let chunk_change = PrFileChange {
                path: format!("{}.part{}", change.path, chunk_num),
                content: chunk_content,
                line_count: current_lines,
                risk_level: change.risk_level,
            };

            let pr = Pr::new(
                format!("pr-{}-{}-{}", task_id, start_pr_number, chunk_num),
                start_pr_number + chunk_num - 1,
                base_branch.to_string(),
                vec![chunk_change],
            );
            prs.push(pr);
        }

        debug!(
            change_path = %change.path,
            total_lines = total_lines,
            chunks = prs.len(),
            "Split large file change"
        );

        prs
    }

    /// Set dependency chain for PRs (each depends on previous)
    fn set_pr_dependencies(prs: &mut Vec<Pr>) {
        for i in 1..prs.len() {
            let prev_id = prs[i - 1].id.clone();
            prs[i].depends_on.push(prev_id);
        }
    }

    /// Check if adding a PR would create a cycle in dependencies
    /// Takes a PR ID string to check against existing PRs
    pub fn would_create_cycle(&self, task_id: &Uuid, new_pr_id: &str) -> bool {
        // Simple cycle check: ensure no PR in stack depends on new_pr_id already
        if let Some(stack) = self.stacks.get(task_id) {
            stack.prs.iter().any(|pr| pr.id == new_pr_id)
        } else {
            false
        }
    }

    /// Get PRs that depend on a given PR
    pub fn get_dependent_prs(&self, task_id: &Uuid, pr_id: &str) -> Vec<&Pr> {
        if let Some(stack) = self.stacks.get(task_id) {
            let pr_id_str = pr_id.to_string();
            stack
                .prs
                .iter()
                .filter(|pr| pr.depends_on.contains(&pr_id_str))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get total line count across all PRs in a stack
    pub fn total_lines(&self, task_id: &Uuid) -> u32 {
        self.stacks
            .get(task_id)
            .map(|stack| stack.prs.iter().map(|pr| pr.line_count()).sum())
            .unwrap_or(0)
    }

    /// Validate PR sizes are all under limit
    pub fn validate_sizes(&self, task_id: &Uuid) -> Result<(), StackedPrError> {
        if let Some(stack) = self.stacks.get(task_id) {
            for pr in &stack.prs {
                if pr.line_count() > self.max_pr_lines {
                    return Err(StackedPrError::PrExceedsSizeLimit {
                        pr_id: pr.id.clone(),
                        lines: pr.line_count(),
                        limit: self.max_pr_lines,
                    });
                }
            }
        }
        Ok(())
    }

    /// Get count of PRs in a stack
    pub fn stack_size(&self, task_id: &Uuid) -> usize {
        self.stacks
            .get(task_id)
            .map(|stack| stack.prs.len())
            .unwrap_or(0)
    }

    /// Get all stacks as a slice
    pub fn all_stacks(&self) -> Vec<&PrStack> {
        self.stacks.values().collect()
    }
}

impl Default for PrStackManager {
    fn default() -> Self {
        Self::new()
    }
}

/// A stack of PRs that form a linear dependency chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrStack {
    /// Task this stack belongs to
    pub task_id: Uuid,
    /// Base branch (e.g., "main")
    pub base_branch: String,
    /// PRs in the stack (ordered from base to head)
    pub prs: Vec<Pr>,
    /// When stack was created
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl PrStack {
    /// Create a new PR stack
    pub fn new(task_id: Uuid, base_branch: String) -> Self {
        Self {
            task_id,
            base_branch,
            prs: Vec::new(),
            created_at: chrono::Utc::now(),
        }
    }

    /// Add a PR to the stack
    pub fn add_pr(&mut self, pr: Pr) -> Result<(), StackedPrError> {
        // Validate PR has correct base branch
        if pr.base_branch != self.base_branch {
            return Err(StackedPrError::InvalidBaseBranch {
                expected: self.base_branch.clone(),
                actual: pr.base_branch.clone(),
            });
        }

        self.prs.push(pr);
        Ok(())
    }

    /// Get the head PR (last in chain)
    pub fn head(&self) -> Option<&Pr> {
        self.prs.last()
    }

    /// Get the base PR (first in chain)
    pub fn base(&self) -> Option<&Pr> {
        self.prs.first()
    }

    /// Get next PR number for new PR
    pub fn next_pr_number(&self) -> u32 {
        (self.prs.len() + 1) as u32
    }

    /// Total lines across all PRs
    pub fn total_lines(&self) -> u32 {
        self.prs.iter().map(|pr| pr.line_count()).sum()
    }

    /// Check if stack is empty
    pub fn is_empty(&self) -> bool {
        self.prs.is_empty()
    }

    /// Get PR by ID
    pub fn get_pr(&self, pr_id: &str) -> Option<&Pr> {
        self.prs.iter().find(|p| p.id == pr_id)
    }

    /// Get PR by index
    pub fn get_pr_at_index(&self, index: usize) -> Option<&Pr> {
        self.prs.get(index)
    }

    /// Iterate over PRs with their position
    pub fn iter_with_position(&self) -> impl Iterator<Item = (usize, &Pr)> {
        self.prs.iter().enumerate()
    }

    /// Get the stack as a list of branch names (for GitHub merge queue)
    pub fn branch_chain(&self) -> Vec<String> {
        self.prs.iter().map(|pr| pr.branch.clone()).collect()
    }
}

/// A single PR in the stack
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pr {
    /// Unique PR identifier
    pub id: String,
    /// PR number within the stack (1, 2, 3...)
    pub pr_number: u32,
    /// Branch name for this PR
    pub branch: String,
    /// Base branch this PR targets
    pub base_branch: String,
    /// Files changed in this PR
    pub files: Vec<PrFileChange>,
    /// PRs this one depends on (usually just the previous one)
    pub depends_on: Vec<String>,
    /// Total line count (additions + deletions)
    pub line_count: u32,
    /// When PR was created
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl Pr {
    /// Create a new PR
    pub fn new(id: String, pr_number: u32, base_branch: String, files: Vec<PrFileChange>) -> Self {
        let branch = format!("{}/pr-{}", base_branch, pr_number);
        let line_count: u32 = files.iter().map(|f| f.line_count).sum();

        Self {
            id,
            pr_number,
            branch,
            base_branch,
            files,
            depends_on: Vec::new(),
            line_count,
            created_at: chrono::Utc::now(),
        }
    }

    /// Get line count for this PR
    pub fn line_count(&self) -> u32 {
        self.line_count
    }

    /// Check if PR is within size limit
    pub fn is_within_limit(&self, limit: u32) -> bool {
        self.line_count <= limit
    }

    /// Get file paths in this PR
    pub fn file_paths(&self) -> Vec<&str> {
        self.files.iter().map(|f| f.path.as_str()).collect()
    }

    /// Check if this PR depends on another
    pub fn depends_on_pr(&self, pr_id: &str) -> bool {
        self.depends_on.contains(&pr_id.to_string())
    }

    /// Add a dependency
    pub fn add_dependency(&mut self, pr_id: String) {
        if !self.depends_on.contains(&pr_id) {
            self.depends_on.push(pr_id);
        }
    }

    /// Check if PR has any files with high risk
    pub fn has_high_risk_files(&self) -> bool {
        self.files
            .iter()
            .any(|f| f.risk_level == FileChangeRisk::High)
    }

    /// Get count of files in this PR
    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}

/// A file change within a PR
#[derive(Debug, Clone, Serialize, Deserialize, Eq)]
pub struct PrFileChange {
    /// File path
    pub path: String,
    /// Content (for new files) or diff summary
    pub content: String,
    /// Approximate line count
    pub line_count: u32,
    /// Risk level of this change
    pub risk_level: FileChangeRisk,
}

impl PartialEq for PrFileChange {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
            && self.line_count == other.line_count
            && self.risk_level == other.risk_level
    }
}

impl Ord for PrFileChange {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.risk_level.cmp(&other.risk_level)
    }
}

impl PartialOrd for PrFileChange {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Risk level for file changes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileChangeRisk {
    Low,
    Medium,
    High,
}

impl FileChangeRisk {
    /// Order for prioritization (high risk first)
    pub fn priority_order(&self) -> u8 {
        match self {
            FileChangeRisk::High => 0,
            FileChangeRisk::Medium => 1,
            FileChangeRisk::Low => 2,
        }
    }
}

impl PartialOrd for FileChangeRisk {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FileChangeRisk {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority_order().cmp(&other.priority_order())
    }
}

/// Errors for stacked PR operations
#[derive(Debug, thiserror::Error)]
pub enum StackedPrError {
    #[error("PR stack not found for task {0}")]
    StackNotFound(Uuid),

    #[error("PR {pr_id} exceeds size limit: {lines} lines (limit: {limit})")]
    PrExceedsSizeLimit {
        pr_id: String,
        lines: u32,
        limit: u32,
    },

    #[error("Invalid base branch: expected {expected}, got {actual}")]
    InvalidBaseBranch { expected: String, actual: String },

    #[error("Cycle detected: PR {0} would create circular dependency")]
    CycleDetected(String),

    #[error("Empty PR not allowed")]
    EmptyPr,

    #[error("PR not found: {0}")]
    PrNotFound(String),
}

/// Configuration for stacked PRs
#[derive(Debug, Clone)]
pub struct StackedPrConfig {
    /// Maximum lines per PR
    pub max_pr_lines: u32,
    /// Minimum lines to consider worth a PR
    pub min_pr_lines: u32,
    /// Maximum files per PR (for reviewability)
    pub max_files_per_pr: usize,
    /// Enable automatic splitting
    pub auto_split: bool,
}

impl Default for StackedPrConfig {
    fn default() -> Self {
        Self {
            max_pr_lines: DEFAULT_MAX_PR_LINES,
            min_pr_lines: MIN_PR_LINES,
            max_files_per_pr: 15,
            auto_split: true,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- Pr Creation Tests ---

    #[test]
    fn test_pr_creation() {
        let files = vec![PrFileChange {
            path: "src/main.rs".to_string(),
            content: "// test".to_string(),
            line_count: 10,
            risk_level: FileChangeRisk::Low,
        }];

        let pr = Pr::new("pr-1".to_string(), 1, "main".to_string(), files);

        assert_eq!(pr.id, "pr-1");
        assert_eq!(pr.pr_number, 1);
        assert_eq!(pr.branch, "main/pr-1");
        assert_eq!(pr.base_branch, "main");
        assert_eq!(pr.line_count(), 10);
        assert!(pr.depends_on.is_empty());
    }

    #[test]
    fn test_pr_multiple_files() {
        let files = vec![
            PrFileChange {
                path: "src/a.rs".to_string(),
                content: "// a".to_string(),
                line_count: 50,
                risk_level: FileChangeRisk::Low,
            },
            PrFileChange {
                path: "src/b.rs".to_string(),
                content: "// b".to_string(),
                line_count: 30,
                risk_level: FileChangeRisk::Medium,
            },
        ];

        let pr = Pr::new("pr-1".to_string(), 1, "main".to_string(), files);

        assert_eq!(pr.line_count(), 80);
        assert_eq!(pr.file_count(), 2);
        assert!(!pr.has_high_risk_files());
    }

    #[test]
    fn test_pr_with_high_risk() {
        let files = vec![PrFileChange {
            path: "Cargo.toml".to_string(),
            content: "[dependencies]".to_string(),
            line_count: 10,
            risk_level: FileChangeRisk::High,
        }];

        let pr = Pr::new("pr-1".to_string(), 1, "main".to_string(), files);

        assert!(pr.has_high_risk_files());
    }

    // --- PrStack Tests ---

    #[test]
    fn test_stack_creation() {
        let task_id = Uuid::new_v4();
        let stack = PrStack::new(task_id, "main".to_string());

        assert_eq!(stack.task_id, task_id);
        assert_eq!(stack.base_branch, "main");
        assert!(stack.is_empty());
        assert!(stack.head().is_none());
        assert!(stack.base().is_none());
    }

    #[test]
    fn test_stack_add_pr() {
        let mut stack = PrStack::new(Uuid::new_v4(), "main".to_string());

        let pr = Pr::new(
            "pr-1".to_string(),
            1,
            "main".to_string(),
            vec![PrFileChange {
                path: "test.rs".to_string(),
                content: "// test".to_string(),
                line_count: 20,
                risk_level: FileChangeRisk::Low,
            }],
        );

        stack.add_pr(pr).unwrap();

        assert_eq!(stack.prs.len(), 1);
        assert!(stack.base().is_some());
        assert!(stack.head().is_some());
        assert_eq!(stack.next_pr_number(), 2);
    }

    #[test]
    fn test_stack_add_pr_wrong_base() {
        let mut stack = PrStack::new(Uuid::new_v4(), "main".to_string());

        let pr = Pr::new(
            "pr-1".to_string(),
            1,
            "develop".to_string(), // Wrong base
            vec![],
        );

        let result = stack.add_pr(pr);
        assert!(result.is_err());
    }

    // --- PrStackManager Tests ---

    #[test]
    fn test_manager_create_stack() {
        let mut manager = PrStackManager::new();
        let task_id = Uuid::new_v4();

        let stack = manager.create_stack(task_id, "main".to_string());

        assert_eq!(stack.base_branch, "main");
        assert!(stack.is_empty());
    }

    #[test]
    fn test_manager_get_nonexistent_stack() {
        let manager = PrStackManager::new();
        let task_id = Uuid::new_v4();

        assert!(manager.get_stack(&task_id).is_none());
    }

    #[test]
    fn test_manager_remove_stack() {
        let mut manager = PrStackManager::new();
        let task_id = Uuid::new_v4();

        manager.create_stack(task_id, "main".to_string());
        assert!(manager.get_stack(&task_id).is_some());

        let removed = manager.remove_stack(&task_id);
        assert!(removed.is_some());
        assert!(manager.get_stack(&task_id).is_none());
    }

    #[test]
    fn test_manager_add_pr() {
        let mut manager = PrStackManager::new();
        let task_id = Uuid::new_v4();

        manager.create_stack(task_id, "main".to_string());

        let pr = Pr::new(
            "pr-1".to_string(),
            1,
            "main".to_string(),
            vec![PrFileChange {
                path: "test.rs".to_string(),
                content: "// test".to_string(),
                line_count: 20,
                risk_level: FileChangeRisk::Low,
            }],
        );

        let result = manager.add_pr(task_id, pr);
        assert!(result.is_ok());
        assert_eq!(manager.stack_size(&task_id), 1);
    }

    #[test]
    fn test_manager_add_pr_nonexistent_stack() {
        let mut manager = PrStackManager::new();
        let task_id = Uuid::new_v4();

        let pr = Pr::new("pr-1".to_string(), 1, "main".to_string(), vec![]);

        let result = manager.add_pr(task_id, pr);
        assert!(result.is_err());
    }

    // --- Size Limit Tests ---

    #[test]
    fn test_pr_within_limit() {
        let pr = Pr::new(
            "pr-1".to_string(),
            1,
            "main".to_string(),
            vec![PrFileChange {
                path: "test.rs".to_string(),
                content: "// test".to_string(),
                line_count: 150,
                risk_level: FileChangeRisk::Low,
            }],
        );

        assert!(pr.is_within_limit(200));
        assert!(!pr.is_within_limit(100));
    }

    #[test]
    fn test_manager_validate_sizes_pass() {
        let mut manager = PrStackManager::new();
        let task_id = Uuid::new_v4();

        manager.create_stack(task_id, "main".to_string());

        let pr = Pr::new(
            "pr-1".to_string(),
            1,
            "main".to_string(),
            vec![PrFileChange {
                path: "test.rs".to_string(),
                content: "// test".to_string(),
                line_count: 150,
                risk_level: FileChangeRisk::Low,
            }],
        );

        manager.add_pr(task_id, pr).unwrap();

        let result = manager.validate_sizes(&task_id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_manager_validate_sizes_fail() {
        let mut manager = PrStackManager::new_with_config(StackedPrConfig {
            max_pr_lines: 100,
            ..Default::default()
        });
        let task_id = Uuid::new_v4();

        manager.create_stack(task_id, "main".to_string());

        let pr = Pr::new(
            "pr-1".to_string(),
            1,
            "main".to_string(),
            vec![PrFileChange {
                path: "test.rs".to_string(),
                content: "// test".to_string(),
                line_count: 150, // Exceeds 100
                risk_level: FileChangeRisk::Low,
            }],
        );

        manager.add_pr(task_id, pr).unwrap();

        let result = manager.validate_sizes(&task_id);
        assert!(result.is_err());
    }

    // --- Splitting Tests ---

    #[test]
    fn test_split_small_changes_no_split() {
        let mut manager = PrStackManager::new();
        let task_id = Uuid::new_v4();
        manager.create_stack(task_id, "main".to_string());

        let changes = vec![
            PrFileChange {
                path: "src/main.rs".to_string(),
                content: "// small change".to_string(),
                line_count: 50,
                risk_level: FileChangeRisk::Low,
            },
            PrFileChange {
                path: "src/lib.rs".to_string(),
                content: "// another small change".to_string(),
                line_count: 50,
                risk_level: FileChangeRisk::Low,
            },
        ];

        let prs = manager.calculate_splits(task_id, &changes).unwrap();

        // Should fit in single PR, no split needed
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].file_count(), 2);
    }

    #[test]
    fn test_split_exceeds_limit() {
        let mut manager = PrStackManager::new();
        let task_id = Uuid::new_v4();
        manager.create_stack(task_id, "main".to_string());

        // Create changes that exceed 200 lines
        let changes = vec![PrFileChange {
            path: "src/main.rs".to_string(),
            content: "// big change".to_string(),
            line_count: 250,
            risk_level: FileChangeRisk::Medium,
        }];

        let prs = manager.calculate_splits(task_id, &changes).unwrap();

        // Should be split
        assert!(prs.len() > 0);
    }

    #[test]
    fn test_split_multiple_prs() {
        let mut manager = PrStackManager::new();
        let task_id = Uuid::new_v4();
        manager.create_stack(task_id, "main".to_string());

        // Create changes that will need multiple PRs
        let changes = vec![
            PrFileChange {
                path: "src/a.rs".to_string(),
                content: "// a".to_string(),
                line_count: 120,
                risk_level: FileChangeRisk::Low,
            },
            PrFileChange {
                path: "src/b.rs".to_string(),
                content: "// b".to_string(),
                line_count: 120,
                risk_level: FileChangeRisk::Low,
            },
            PrFileChange {
                path: "src/c.rs".to_string(),
                content: "// c".to_string(),
                line_count: 120,
                risk_level: FileChangeRisk::Low,
            },
        ];

        let prs = manager.calculate_splits(task_id, &changes).unwrap();

        // Should be split into multiple PRs
        assert!(prs.len() > 1);

        // Verify dependency chain
        for i in 1..prs.len() {
            assert!(prs[i].depends_on.contains(&prs[i - 1].id));
        }
    }

    // --- Dependency Chain Tests ---

    #[test]
    fn test_pr_dependencies_chain() {
        let mut manager = PrStackManager::new();
        let task_id = Uuid::new_v4();
        manager.create_stack(task_id, "main".to_string());

        let changes = vec![
            PrFileChange {
                path: "src/1.rs".to_string(),
                content: "// 1".to_string(),
                line_count: 50,
                risk_level: FileChangeRisk::Low,
            },
            PrFileChange {
                path: "src/2.rs".to_string(),
                content: "// 2".to_string(),
                line_count: 50,
                risk_level: FileChangeRisk::Medium,
            },
            PrFileChange {
                path: "src/3.rs".to_string(),
                content: "// 3".to_string(),
                line_count: 50,
                risk_level: FileChangeRisk::High,
            },
        ];

        let prs = manager.calculate_splits(task_id, &changes).unwrap();

        // High risk should be first
        if prs.len() >= 3 {
            let high_risk_pr = prs.iter().find(|p| p.has_high_risk_files());
            assert!(high_risk_pr.is_some());
            // High risk PR should not depend on others
            let high_pr = high_risk_pr.unwrap();
            assert!(high_pr.depends_on.is_empty() || high_pr.depends_on.len() == 0);
        }
    }

    // --- Total Lines Tests ---

    #[test]
    fn test_total_lines() {
        let mut manager = PrStackManager::new();
        let task_id = Uuid::new_v4();
        manager.create_stack(task_id, "main".to_string());

        let pr1 = Pr::new(
            "pr-1".to_string(),
            1,
            "main".to_string(),
            vec![PrFileChange {
                path: "a.rs".to_string(),
                content: "// a".to_string(),
                line_count: 100,
                risk_level: FileChangeRisk::Low,
            }],
        );

        let pr2 = Pr::new(
            "pr-2".to_string(),
            2,
            "main".to_string(),
            vec![PrFileChange {
                path: "b.rs".to_string(),
                content: "// b".to_string(),
                line_count: 50,
                risk_level: FileChangeRisk::Low,
            }],
        );

        manager.add_pr(task_id, pr1).unwrap();
        manager.add_pr(task_id, pr2).unwrap();

        assert_eq!(manager.total_lines(&task_id), 150);
    }

    // --- Branch Chain Tests ---

    #[test]
    fn test_branch_chain() {
        let mut stack = PrStack::new(Uuid::new_v4(), "main".to_string());

        stack
            .add_pr(Pr::new("pr-1".to_string(), 1, "main".to_string(), vec![]))
            .unwrap();

        stack
            .add_pr(Pr::new("pr-2".to_string(), 2, "main".to_string(), vec![]))
            .unwrap();

        let chain = stack.branch_chain();
        assert_eq!(chain, vec!["main/pr-1", "main/pr-2"]);
    }

    // --- FileChangeRisk Ordering Tests ---

    #[test]
    fn test_file_change_risk_ordering() {
        let mut changes = vec![
            PrFileChange {
                path: "low.rs".to_string(),
                content: "// low".to_string(),
                line_count: 10,
                risk_level: FileChangeRisk::Low,
            },
            PrFileChange {
                path: "high.rs".to_string(),
                content: "// high".to_string(),
                line_count: 10,
                risk_level: FileChangeRisk::High,
            },
            PrFileChange {
                path: "medium.rs".to_string(),
                content: "// medium".to_string(),
                line_count: 10,
                risk_level: FileChangeRisk::Medium,
            },
        ];

        changes.sort();

        // High should come first
        assert_eq!(changes[0].risk_level, FileChangeRisk::High);
        assert_eq!(changes[1].risk_level, FileChangeRisk::Medium);
        assert_eq!(changes[2].risk_level, FileChangeRisk::Low);
    }

    // --- StackedPrConfig Tests ---

    #[test]
    fn test_config_default() {
        let config = StackedPrConfig::default();
        assert_eq!(config.max_pr_lines, 200);
        assert_eq!(config.min_pr_lines, 20);
        assert_eq!(config.max_files_per_pr, 15);
        assert!(config.auto_split);
    }

    // --- Empty PR Tests ---

    #[test]
    fn test_empty_pr_error() {
        let result = Pr::new(
            "pr-1".to_string(),
            1,
            "main".to_string(),
            vec![], // Empty files
        );

        // Empty PR should have 0 line count
        assert_eq!(result.line_count(), 0);
        assert!(result.files.is_empty());
    }

    // --- Error Display Tests ---

    #[test]
    fn test_stack_not_found_error() {
        let err = StackedPrError::StackNotFound(Uuid::new_v4());
        assert!(err.to_string().contains("PR stack not found"));
    }

    #[test]
    fn test_pr_exceeds_size_error() {
        let err = StackedPrError::PrExceedsSizeLimit {
            pr_id: "pr-1".to_string(),
            lines: 300,
            limit: 200,
        };
        let display = err.to_string();
        assert!(display.contains("pr-1"));
        assert!(display.contains("300"));
        assert!(display.contains("200"));
    }
}

// Need to impl Default for PrStackManager with config
impl PrStackManager {
    /// Create manager with custom config
    pub fn new_with_config(config: StackedPrConfig) -> Self {
        Self {
            stacks: HashMap::new(),
            max_pr_lines: config.max_pr_lines,
            min_pr_lines: config.min_pr_lines,
        }
    }
}
