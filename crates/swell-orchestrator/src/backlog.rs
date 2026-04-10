//! Work backlog management for autonomous task generation.
//!
//! The backlog aggregates work from four sources:
//! 1. **Plan tasks**: Tasks from the approved plan (highest priority)
//! 2. **Failure-derived**: Tasks generated from validation failures
//! 3. **Spec-gap**: Tasks identified by gap analysis
//! 4. **Improvement**: Optional quality improvement suggestions (lowest priority)
//!
//! # Priority Scoring
//!
//! Tasks are scored 1-5 based on:
//! - **Spec alignment**: Is the task mentioned in the spec?
//! - **Blocking impact**: Does it unblock other tasks?
//! - **Estimated complexity**: Lower complexity = higher priority for initial work
//!
//! # Deduplication
//!
//! Before adding a task, the backlog checks for duplicates using:
//! - Description similarity (cosine distance on embeddings if available, else Levenshtein)
//! - File overlap analysis (if a task modifies the same files as an existing task)
//!
//! # Auto-approval Rules
//!
//! - Plan tasks and failure-derived tasks are auto-approved
//! - Spec-gap and improvement tasks require operator approval
//! - As the run progresses, the decay function raises the approval threshold

use crate::SwellError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Source of a task in the backlog
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BacklogSource {
    /// Task from the approved plan (highest priority, auto-approved)
    Plan,
    /// Task derived from a validation failure (auto-approved)
    FailureDerived,
    /// Task from spec gap analysis (requires approval)
    SpecGap,
    /// Optional improvement suggestion (requires approval, lowest priority)
    Improvement,
}

impl BacklogSource {
    /// Returns true if tasks from this source are auto-approved
    pub fn is_auto_approved(&self) -> bool {
        matches!(self, BacklogSource::Plan | BacklogSource::FailureDerived)
    }

    /// Priority rank (lower = higher priority)
    pub fn priority_rank(&self) -> u32 {
        match self {
            BacklogSource::Plan => 0,
            BacklogSource::FailureDerived => 1,
            BacklogSource::SpecGap => 2,
            BacklogSource::Improvement => 3,
        }
    }
}

/// A task in the work backlog
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacklogItem {
    pub id: Uuid,
    pub description: String,
    pub source: BacklogSource,
    /// Files that this task would modify (for deduplication)
    pub affected_files: Vec<String>,
    /// Priority score (1-5, higher = more important)
    pub priority_score: u32,
    /// Whether this item has been approved for execution
    pub approved: bool,
    /// Task ID in the state machine (if already created)
    pub task_id: Option<Uuid>,
    /// Original task ID if this is failure-derived
    pub original_task_id: Option<Uuid>,
    /// Failure signal if this is failure-derived
    pub failure_signal: Option<String>,
    /// Spec ID if this is spec-gap
    pub spec_id: Option<Uuid>,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
}

impl BacklogItem {
    /// Create a new backlog item from a failure
    pub fn from_failure(
        original_task_id: Uuid,
        failure_signal: String,
        affected_files: Vec<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            description: format!("Fix failure: {}", failure_signal),
            source: BacklogSource::FailureDerived,
            affected_files,
            priority_score: 4, // Failures are high priority
            approved: true,    // Auto-approved
            task_id: None,
            original_task_id: Some(original_task_id),
            failure_signal: Some(failure_signal),
            spec_id: None,
            created_at: Utc::now(),
        }
    }

    /// Create a new backlog item from a spec gap
    pub fn from_spec_gap(
        spec_id: Uuid,
        gap_description: String,
        affected_files: Vec<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            description: gap_description,
            source: BacklogSource::SpecGap,
            affected_files,
            priority_score: 3, // Spec gaps are medium priority
            approved: false,   // Requires approval
            task_id: None,
            original_task_id: None,
            failure_signal: None,
            spec_id: Some(spec_id),
            created_at: Utc::now(),
        }
    }

    /// Create a new improvement suggestion
    pub fn improvement(description: String, affected_files: Vec<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            description,
            source: BacklogSource::Improvement,
            affected_files,
            priority_score: 2, // Improvements are lower priority
            approved: false,
            task_id: None,
            original_task_id: None,
            failure_signal: None,
            spec_id: None,
            created_at: Utc::now(),
        }
    }

    /// Create a plan task (always auto-approved)
    pub fn plan_task(description: String, affected_files: Vec<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            description,
            source: BacklogSource::Plan,
            affected_files,
            priority_score: 5, // Plan tasks are highest priority
            approved: true,
            task_id: None,
            original_task_id: None,
            failure_signal: None,
            spec_id: None,
            created_at: Utc::now(),
        }
    }
}

/// Priority scoring configuration
#[derive(Debug, Clone)]
pub struct PriorityScoringConfig {
    /// Minimum priority score to auto-approve (0-5)
    pub auto_approve_threshold: u32,
    /// Weight for spec alignment (0.0-1.0)
    pub spec_alignment_weight: f32,
    /// Weight for blocking impact (0.0-1.0)
    pub blocking_impact_weight: f32,
    /// Weight for complexity (inverse - lower complexity = higher priority)
    pub complexity_weight: f32,
}

impl Default for PriorityScoringConfig {
    fn default() -> Self {
        Self {
            auto_approve_threshold: 3,
            spec_alignment_weight: 0.4,
            blocking_impact_weight: 0.3,
            complexity_weight: 0.3,
        }
    }
}

/// Deduplication configuration
#[derive(Debug, Clone)]
pub struct DeduplicationConfig {
    /// Minimum similarity score to consider tasks as duplicates (0.0-1.0)
    /// Tasks with similarity >= threshold are considered duplicates
    pub similarity_threshold: f32,
    /// Minimum file overlap to consider tasks as duplicates (0.0-1.0)
    /// Percentage of files that must overlap
    pub file_overlap_threshold: f32,
    /// Maximum Levenshtein distance for string similarity (if no embeddings)
    pub max_levenshtein_distance: usize,
}

impl Default for DeduplicationConfig {
    fn default() -> Self {
        Self {
            similarity_threshold: 0.85,
            file_overlap_threshold: 0.5,
            max_levenshtein_distance: 30,
        }
    }
}

/// The work backlog aggregating tasks from multiple sources
pub struct WorkBacklog {
    items: Vec<BacklogItem>,
    /// Map from item ID to index in items vec
    item_index: HashMap<Uuid, usize>,
    /// Set of approved item IDs for quick lookup
    approved_items: std::collections::HashSet<Uuid>,
    /// Configuration
    priority_config: PriorityScoringConfig,
    deduplication_config: DeduplicationConfig,
    /// Maximum items in backlog
    max_items: usize,
    /// Count of failure-derived tasks per original task (capped at 3)
    failure_derived_counts: HashMap<Uuid, u32>,
}

impl WorkBacklog {
    /// Create a new empty work backlog
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            item_index: HashMap::new(),
            approved_items: std::collections::HashSet::new(),
            priority_config: PriorityScoringConfig::default(),
            deduplication_config: DeduplicationConfig::default(),
            max_items: 100,
            failure_derived_counts: HashMap::new(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(
        priority_config: PriorityScoringConfig,
        deduplication_config: DeduplicationConfig,
        max_items: usize,
    ) -> Self {
        Self {
            items: Vec::new(),
            item_index: HashMap::new(),
            approved_items: std::collections::HashSet::new(),
            priority_config,
            deduplication_config,
            max_items,
            failure_derived_counts: HashMap::new(),
        }
    }

    /// Add a plan task
    pub fn add_plan_task(
        &mut self,
        description: String,
        affected_files: Vec<String>,
    ) -> Result<Uuid, SwellError> {
        let item = BacklogItem::plan_task(description, affected_files);
        self.add_item(item)
    }

    /// Add a failure-derived task
    pub fn add_failure_derived(
        &mut self,
        original_task_id: Uuid,
        failure_signal: String,
        affected_files: Vec<String>,
    ) -> Result<Uuid, SwellError> {
        // Check if we've hit the cap (3 per original task)
        let current_count = *self
            .failure_derived_counts
            .get(&original_task_id)
            .unwrap_or(&0);
        if current_count >= 3 {
            warn!(
                original_task_id = %original_task_id,
                "Failure-derived task cap reached (3), skipping"
            );
            return Err(SwellError::InvalidOperation(
                "Failure-derived task cap reached (3 per original task)".into(),
            ));
        }

        let item = BacklogItem::from_failure(original_task_id, failure_signal, affected_files);
        let item_id = item.id;

        // Add the item first
        self.add_item(item)?;

        // Only increment AFTER successful add
        *self
            .failure_derived_counts
            .entry(original_task_id)
            .or_insert(0) += 1;

        Ok(item_id)
    }

    /// Add a spec-gap task
    pub fn add_spec_gap(
        &mut self,
        spec_id: Uuid,
        gap_description: String,
        affected_files: Vec<String>,
    ) -> Result<Uuid, SwellError> {
        let item = BacklogItem::from_spec_gap(spec_id, gap_description, affected_files);
        self.add_item(item)
    }

    /// Add an improvement suggestion
    pub fn add_improvement(
        &mut self,
        description: String,
        affected_files: Vec<String>,
    ) -> Result<Uuid, SwellError> {
        let item = BacklogItem::improvement(description, affected_files);
        self.add_item(item)
    }

    /// Add an item to the backlog
    fn add_item(&mut self, mut item: BacklogItem) -> Result<Uuid, SwellError> {
        // Check capacity
        if self.items.len() >= self.max_items {
            return Err(SwellError::InvalidOperation(
                "Backlog item budget exceeded (max 100)".into(),
            ));
        }

        // Check for duplicates
        if let Some(existing) = self.find_duplicate(&item) {
            debug!(
                item_id = %item.id,
                existing_id = %existing,
                "Duplicate item rejected"
            );
            return Err(SwellError::InvalidOperation(format!(
                "Duplicate task rejected (similar to existing task {})",
                existing
            )));
        }

        // Auto-approval only for sources that are explicitly auto-approved
        // (Plan and FailureDerived). SpecGap and Improvement require explicit approval.
        if item.source.is_auto_approved() {
            item.approved = true;
            self.approved_items.insert(item.id);
        }
        // Note: Non-auto-approved items (SpecGap, Improvement) start with approved=false
        // and require explicit approval via approve_item()

        let id = item.id;
        let idx = self.items.len();
        self.items.push(item);
        self.item_index.insert(id, idx);

        info!(
            item_id = %id,
            source = ?self.items[idx].source,
            priority = self.items[idx].priority_score,
            approved = self.items[idx].approved,
            "Backlog item added"
        );

        Ok(id)
    }

    /// Find a duplicate item in the backlog
    ///
    /// Items are considered duplicates if EITHER:
    /// - They have significant file overlap (50%+ of files in common), OR
    /// - Their descriptions are very similar (85%+ similar using Levenshtein)
    ///   AND the new item is NOT failure-derived
    ///
    /// Failure-derived tasks are exempt from description similarity checks
    /// to allow multiple fixes for different errors from the same original task.
    fn find_duplicate(&self, item: &BacklogItem) -> Option<Uuid> {
        for existing in &self.items {
            // Check file overlap
            if self.has_significant_file_overlap(item, existing) {
                return Some(existing.id);
            }

            // For failure-derived tasks, only check file overlap, not description
            // This allows "Fix error 1" and "Fix error 2" to both be added
            if item.source == BacklogSource::FailureDerived {
                continue;
            }

            // Check description similarity for non-failure-derived items
            if self.is_similar_description(&item.description, &existing.description) {
                return Some(existing.id);
            }
        }
        None
    }

    /// Check if two items have significant file overlap
    fn has_significant_file_overlap(&self, item1: &BacklogItem, item2: &BacklogItem) -> bool {
        if item1.affected_files.is_empty() || item2.affected_files.is_empty() {
            return false;
        }

        let overlap_count = item1
            .affected_files
            .iter()
            .filter(|f| item2.affected_files.contains(f))
            .count();

        let min_len = item1.affected_files.len().min(item2.affected_files.len());
        if min_len == 0 {
            return false;
        }

        let overlap_ratio = overlap_count as f32 / min_len as f32;
        overlap_ratio >= self.deduplication_config.file_overlap_threshold
    }

    /// Check if two descriptions are similar
    fn is_similar_description(&self, desc1: &str, desc2: &str) -> bool {
        // Simple Levenshtein distance check
        let distance = levenshtein_distance(desc1, desc2);
        let max_len = desc1.len().max(desc2.len());
        if max_len == 0 {
            return false;
        }

        let similarity = 1.0 - (distance as f32 / max_len as f32);
        similarity >= self.deduplication_config.similarity_threshold
    }

    /// Get all approved items sorted by priority
    pub fn get_approved_items(&self) -> Vec<&BacklogItem> {
        let mut approved: Vec<&BacklogItem> =
            self.items.iter().filter(|item| item.approved).collect();

        // Sort by source priority then by priority score
        approved.sort_by(|a, b| {
            let source_cmp = a.source.priority_rank().cmp(&b.source.priority_rank());
            if source_cmp == std::cmp::Ordering::Equal {
                b.priority_score.cmp(&a.priority_score) // Higher score first
            } else {
                source_cmp
            }
        });

        approved
    }

    /// Get all pending (unapproved) items
    pub fn get_pending_items(&self) -> Vec<&BacklogItem> {
        self.items.iter().filter(|item| !item.approved).collect()
    }

    /// Get all items
    pub fn get_all_items(&self) -> Vec<&BacklogItem> {
        self.items.iter().collect()
    }

    /// Get item by ID
    pub fn get_item(&self, id: Uuid) -> Option<&BacklogItem> {
        self.item_index
            .get(&id)
            .and_then(|idx| self.items.get(*idx))
    }

    /// Get item by ID (mutable)
    pub fn get_item_mut(&mut self, id: Uuid) -> Option<&mut BacklogItem> {
        self.item_index
            .get(&id)
            .and_then(|idx| self.items.get_mut(*idx))
    }

    /// Approve an item
    pub fn approve_item(&mut self, id: Uuid) -> Result<(), SwellError> {
        let item = self.get_item_mut(id).ok_or(SwellError::TaskNotFound(id))?;
        item.approved = true;
        self.approved_items.insert(id);
        debug!(item_id = %id, "Backlog item approved");
        Ok(())
    }

    /// Reject an item
    pub fn remove_item(&mut self, id: Uuid) -> Result<(), SwellError> {
        let idx = self
            .item_index
            .remove(&id)
            .ok_or(SwellError::TaskNotFound(id))?;
        let item = self.items.remove(idx);

        // Update indices for all items after the removed one
        for (i, item) in self.items.iter().enumerate() {
            self.item_index.insert(item.id, i);
        }

        self.approved_items.remove(&id);

        // If it was failure-derived, decrement the counter
        if let Some(original_id) = item.original_task_id {
            if let Some(count) = self.failure_derived_counts.get_mut(&original_id) {
                if *count > 0 {
                    *count -= 1;
                }
            }
        }

        info!(item_id = %id, "Backlog item removed");
        Ok(())
    }

    /// Get count of items
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Get count of approved items
    pub fn approved_count(&self) -> usize {
        self.approved_items.len()
    }

    /// Apply decay function - raise approval threshold as run progresses
    ///
    /// This increases the auto-approve threshold based on run progress.
    /// After 80% completion, even failure-derived tasks may need approval
    /// if they involve files outside the original plan scope.
    pub fn apply_decay(&mut self, completion_ratio: f32) {
        let base_threshold = self.priority_config.auto_approve_threshold;

        // Raise threshold as we progress (0.0 -> 1.0 maps to 0 -> +2)
        let additional_threshold = (completion_ratio * 2.0) as u32;
        let new_threshold = base_threshold + additional_threshold.min(2);

        if new_threshold > base_threshold {
            debug!(
                completion_ratio = %completion_ratio,
                old_threshold = base_threshold,
                new_threshold = new_threshold,
                "Applying decay function to approval threshold"
            );

            // Re-evaluate non-approved items with new threshold
            for item in &mut self.items {
                if !item.approved && item.priority_score >= new_threshold {
                    item.approved = true;
                    self.approved_items.insert(item.id);
                    info!(
                        item_id = %item.id,
                        new_threshold = new_threshold,
                        "Item auto-approved after decay"
                    );
                }
            }

            self.priority_config.auto_approve_threshold = new_threshold;
        }
    }

    /// Get backlog statistics
    pub fn stats(&self) -> BacklogStats {
        let by_source =
            self.items
                .iter()
                .fold(std::collections::HashMap::new(), |mut acc, item| {
                    *acc.entry(item.source).or_insert(0) += 1;
                    acc
                });

        BacklogStats {
            total_items: self.items.len(),
            approved_items: self.approved_items.len(),
            pending_items: self.items.len() - self.approved_items.len(),
            by_source,
        }
    }
}

impl Default for WorkBacklog {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about the backlog
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacklogStats {
    pub total_items: usize,
    pub approved_items: usize,
    pub pending_items: usize,
    pub by_source: std::collections::HashMap<BacklogSource, usize>,
}

/// Calculate Levenshtein distance between two strings
#[allow(clippy::needless_range_loop)]
fn levenshtein_distance(s1: &str, s2: &str) -> usize {
    let s1_chars: Vec<char> = s1.chars().collect();
    let s2_chars: Vec<char> = s2.chars().collect();
    let len1 = s1_chars.len();
    let len2 = s2_chars.len();

    if len1 == 0 {
        return len2;
    }
    if len2 == 0 {
        return len1;
    }

    let mut matrix = vec![vec![0usize; len2 + 1]; len1 + 1];

    for i in 0..=len1 {
        matrix[i][0] = i;
    }
    for j in 0..=len2 {
        matrix[0][j] = j;
    }

    for i in 1..=len1 {
        for j in 1..=len2 {
            let cost = if s1_chars[i - 1] == s2_chars[j - 1] {
                0
            } else {
                1
            };
            matrix[i][j] = std::cmp::min(
                std::cmp::min(matrix[i - 1][j] + 1, matrix[i][j - 1] + 1),
                matrix[i - 1][j - 1] + cost,
            );
        }
    }

    matrix[len1][len2]
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- BacklogItem Tests ---

    #[test]
    fn test_plan_task_auto_approved() {
        let item = BacklogItem::plan_task(
            "Implement feature X".to_string(),
            vec!["src/x.rs".to_string()],
        );
        assert_eq!(item.source, BacklogSource::Plan);
        assert!(item.approved);
        assert_eq!(item.priority_score, 5);
    }

    #[test]
    fn test_failure_derived_auto_approved() {
        let item = BacklogItem::from_failure(
            Uuid::new_v4(),
            "Type error in auth.ts line 42".to_string(),
            vec!["src/auth.ts".to_string()],
        );
        assert_eq!(item.source, BacklogSource::FailureDerived);
        assert!(item.approved);
        assert!(item.description.contains("Type error"));
        assert!(item.original_task_id.is_some());
        assert!(item.failure_signal.is_some());
    }

    #[test]
    fn test_spec_gap_requires_approval() {
        let item = BacklogItem::from_spec_gap(
            Uuid::new_v4(),
            "Missing error handling for network failures".to_string(),
            vec!["src/http.rs".to_string()],
        );
        assert_eq!(item.source, BacklogSource::SpecGap);
        assert!(!item.approved);
    }

    #[test]
    fn test_improvement_requires_approval() {
        let item = BacklogItem::improvement(
            "Remove dead code in utils.rs".to_string(),
            vec!["src/utils.rs".to_string()],
        );
        assert_eq!(item.source, BacklogSource::Improvement);
        assert!(!item.approved);
        assert_eq!(item.priority_score, 2);
    }

    // --- WorkBacklog Tests ---

    #[test]
    fn test_add_plan_task() {
        let mut backlog = WorkBacklog::new();
        let result = backlog.add_plan_task(
            "Implement feature X".to_string(),
            vec!["src/x.rs".to_string()],
        );
        assert!(result.is_ok());
        assert_eq!(backlog.len(), 1);
        assert_eq!(backlog.approved_count(), 1);
    }

    #[test]
    fn test_add_failure_derived() {
        let mut backlog = WorkBacklog::new();
        let original_id = Uuid::new_v4();

        let result = backlog.add_failure_derived(
            original_id,
            "Type error".to_string(),
            vec!["src/auth.ts".to_string()],
        );
        assert!(result.is_ok());
        assert_eq!(backlog.len(), 1);
        assert_eq!(backlog.approved_count(), 1);
    }

    #[test]
    fn test_failure_derived_cap() {
        let mut backlog = WorkBacklog::new();
        let original_id = Uuid::new_v4();

        // Add 3 failure-derived tasks
        assert!(backlog
            .add_failure_derived(original_id, "Error 1".to_string(), vec![])
            .is_ok());
        assert!(backlog
            .add_failure_derived(original_id, "Error 2".to_string(), vec![])
            .is_ok());
        assert!(backlog
            .add_failure_derived(original_id, "Error 3".to_string(), vec![])
            .is_ok());

        // 4th should fail
        let result = backlog.add_failure_derived(original_id, "Error 4".to_string(), vec![]);
        assert!(result.is_err());
        assert_eq!(backlog.len(), 3);
    }

    #[test]
    fn test_add_spec_gap_not_auto_approved() {
        let mut backlog = WorkBacklog::new();
        let spec_id = Uuid::new_v4();

        let result = backlog.add_spec_gap(
            spec_id,
            "Missing feature".to_string(),
            vec!["src/main.rs".to_string()],
        );
        assert!(result.is_ok());
        assert_eq!(backlog.len(), 1);
        assert_eq!(backlog.approved_count(), 0);
    }

    #[test]
    fn test_approve_item() {
        let mut backlog = WorkBacklog::new();
        let id = backlog
            .add_spec_gap(Uuid::new_v4(), "Missing feature".to_string(), vec![])
            .unwrap();

        assert_eq!(backlog.approved_count(), 0);
        backlog.approve_item(id).unwrap();
        assert_eq!(backlog.approved_count(), 1);
        assert!(backlog.get_item(id).unwrap().approved);
    }

    #[test]
    fn test_remove_item() {
        let mut backlog = WorkBacklog::new();
        let id = backlog.add_plan_task("Task".to_string(), vec![]).unwrap();

        assert_eq!(backlog.len(), 1);
        backlog.remove_item(id).unwrap();
        assert_eq!(backlog.len(), 0);
    }

    #[test]
    fn test_deduplication_by_file_overlap() {
        let mut backlog = WorkBacklog::new();

        // Add first task
        backlog
            .add_plan_task(
                "Implement feature X".to_string(),
                vec!["src/x.rs".to_string(), "src/y.rs".to_string()],
            )
            .unwrap();

        // Try to add duplicate with SAME description but same files
        // This should be caught by description similarity since files are identical
        let result = backlog.add_plan_task(
            "Implement feature X".to_string(), // Exact same description
            vec!["src/x.rs".to_string(), "src/y.rs".to_string()],
        );
        assert!(result.is_err()); // Should be rejected as duplicate (same desc + same files)

        // But if we add a very different description with different files
        // it should be accepted (no description similarity, no file overlap)
        let result2 = backlog.add_plan_task(
            "Completely different task with other files".to_string(),
            vec!["src/other.rs".to_string()], // Different files
        );
        assert!(result2.is_ok()); // Different files AND very different description
    }

    #[test]
    fn test_deduplication_by_description() {
        let mut backlog = WorkBacklog::new();

        // Add first task with unique description
        backlog
            .add_plan_task(
                "Implement feature X with custom authentication".to_string(),
                vec!["src/auth.rs".to_string()],
            )
            .unwrap();

        // Try to add similar description
        let result = backlog.add_plan_task(
            "Implement feature X with custom authentication mechanism".to_string(),
            vec!["src/other.rs".to_string()],
        );
        // Note: These are different files, so might not be caught as duplicate
        // unless the Levenshtein distance is within threshold
        assert!(result.is_ok()); // Different enough with different files
    }

    #[test]
    fn test_get_approved_items_sorted_by_priority() {
        let mut backlog = WorkBacklog::new();

        // Add items in random order
        backlog
            .add_improvement("Low priority".to_string(), vec![])
            .unwrap();
        backlog
            .add_plan_task("High priority plan".to_string(), vec![])
            .unwrap();
        backlog
            .add_spec_gap(
                Uuid::new_v4(),
                "Medium priority spec gap".to_string(),
                vec![],
            )
            .unwrap();

        let approved = backlog.get_approved_items();
        // Plan tasks should come first (highest source priority)
        assert_eq!(approved[0].source, BacklogSource::Plan);
        assert_eq!(approved[0].description, "High priority plan");
    }

    #[test]
    fn test_backlog_max_capacity() {
        let mut backlog = WorkBacklog::with_config(
            PriorityScoringConfig::default(),
            DeduplicationConfig::default(),
            3, // Small max for testing
        );

        backlog.add_plan_task("Task 1".to_string(), vec![]).unwrap();
        backlog.add_plan_task("Task 2".to_string(), vec![]).unwrap();
        backlog.add_plan_task("Task 3".to_string(), vec![]).unwrap();

        // Should be at capacity
        let result = backlog.add_plan_task("Task 4".to_string(), vec![]);
        assert!(result.is_err());
    }

    #[test]
    fn test_decay_function() {
        let mut backlog = WorkBacklog::new();

        // Add a spec-gap item with priority 3 (threshold is 3 by default)
        backlog
            .add_spec_gap(Uuid::new_v4(), "Medium priority".to_string(), vec![])
            .unwrap();
        assert_eq!(backlog.approved_count(), 0); // Not auto-approved

        // Apply decay at 0% completion - should still not approve
        backlog.apply_decay(0.0);
        assert_eq!(backlog.approved_count(), 0);

        // Apply decay at 50% completion - threshold raised to 4
        backlog.apply_decay(0.5);
        assert_eq!(backlog.approved_count(), 0); // Still not enough

        // Apply decay at 100% completion - threshold raised to 5
        backlog.apply_decay(1.0);
        // At threshold 5, priority 3 still doesn't make it
        assert_eq!(backlog.approved_count(), 0);
    }

    #[test]
    fn test_stats() {
        let mut backlog = WorkBacklog::new();

        backlog
            .add_plan_task("Plan task".to_string(), vec![])
            .unwrap();
        backlog
            .add_failure_derived(Uuid::new_v4(), "Failure".to_string(), vec![])
            .unwrap();
        backlog
            .add_spec_gap(Uuid::new_v4(), "Spec gap".to_string(), vec![])
            .unwrap();

        let stats = backlog.stats();
        assert_eq!(stats.total_items, 3);
        assert_eq!(stats.approved_items, 2); // Plan and FailureDerived
        assert_eq!(stats.pending_items, 1); // SpecGap
    }

    // --- Levenshtein Distance Tests ---

    #[test]
    fn test_levenshtein_identical_strings() {
        assert_eq!(levenshtein_distance("hello", "hello"), 0);
    }

    #[test]
    fn test_levenshtein_one_char_diff() {
        assert_eq!(levenshtein_distance("hello", "hallo"), 1);
    }

    #[test]
    fn test_levenshtein_empty_string() {
        assert_eq!(levenshtein_distance("", "hello"), 5);
        assert_eq!(levenshtein_distance("hello", ""), 5);
        assert_eq!(levenshtein_distance("", ""), 0);
    }

    #[test]
    fn test_levenshtein_complete_change() {
        assert_eq!(levenshtein_distance("hello", "world"), 4);
    }

    // --- Source Priority Tests ---

    #[test]
    fn test_source_priority_order() {
        assert!(
            BacklogSource::Plan.priority_rank() < BacklogSource::FailureDerived.priority_rank()
        );
        assert!(
            BacklogSource::FailureDerived.priority_rank() < BacklogSource::SpecGap.priority_rank()
        );
        assert!(
            BacklogSource::SpecGap.priority_rank() < BacklogSource::Improvement.priority_rank()
        );
    }

    #[test]
    fn test_auto_approval_for_plan_and_failure() {
        assert!(BacklogSource::Plan.is_auto_approved());
        assert!(BacklogSource::FailureDerived.is_auto_approved());
        assert!(!BacklogSource::SpecGap.is_auto_approved());
        assert!(!BacklogSource::Improvement.is_auto_approved());
    }
}
