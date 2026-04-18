//! Novelty Checker - rejects duplicate tasks by similarity scoring and file overlap analysis.
//!
//! This module provides functionality to:
//! - Compute task similarity based on description text
//! - Analyze file overlap between tasks
//! - Reject duplicate tasks before they enter the system
//!
//! # Deduplication Strategy
//!
//! A task is considered a duplicate if EITHER:
//! - **High description similarity**: The Levenshtein similarity between descriptions is >= 85%
//! - **Significant file overlap**: At least 50% of files overlap with an existing task
//!
//! Failure-derived tasks are exempt from description similarity checks to allow
//! multiple fixes for different errors from the same original task.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use swell_core::TaskId;
use tracing::{debug, info};
use uuid::Uuid;

/// Configuration for novelty checking
#[derive(Debug, Clone)]
pub struct NoveltyCheckerConfig {
    /// Minimum similarity score to consider tasks as duplicates (0.0-1.0)
    /// Tasks with similarity >= threshold are considered duplicates
    pub similarity_threshold: f32,
    /// Minimum file overlap to consider tasks as duplicates (0.0-1.0)
    /// Percentage of files that must overlap
    pub file_overlap_threshold: f32,
    /// Maximum Levenshtein distance for string similarity (if no embeddings)
    pub max_levenshtein_distance: usize,
    /// Whether to enable novelty checking
    pub enabled: bool,
}

impl Default for NoveltyCheckerConfig {
    fn default() -> Self {
        Self {
            similarity_threshold: 0.85,
            file_overlap_threshold: 0.8, // 80% file overlap threshold
            max_levenshtein_distance: 30,
            enabled: true,
        }
    }
}

/// Result of a novelty check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoveltyCheckResult {
    /// Whether the task is novel (not a duplicate)
    pub is_novel: bool,
    /// Similarity score to the most similar existing task (0.0-1.0)
    pub max_similarity: f32,
    /// File overlap ratio with the most overlapping task (0.0-1.0)
    pub max_file_overlap: f32,
    /// ID of the task that makes this a duplicate (if not novel)
    pub duplicate_of: Option<TaskId>,
    /// Reason for rejection (if not novel)
    pub rejection_reason: Option<String>,
}

impl NoveltyCheckResult {
    /// Create a result indicating the task is novel
    pub fn novel(max_similarity: f32, max_file_overlap: f32) -> Self {
        Self {
            is_novel: true,
            max_similarity,
            max_file_overlap,
            duplicate_of: None,
            rejection_reason: None,
        }
    }

    /// Create a result indicating the task is a duplicate
    pub fn duplicate(
        duplicate_of: TaskId,
        reason: String,
        max_similarity: f32,
        max_file_overlap: f32,
    ) -> Self {
        Self {
            is_novel: false,
            max_similarity,
            max_file_overlap,
            duplicate_of: Some(duplicate_of),
            rejection_reason: Some(reason),
        }
    }
}

/// Tracks an existing task for novelty comparison
#[derive(Debug, Clone)]
pub struct TrackedTask {
    pub id: TaskId,
    pub description: String,
    pub affected_files: Vec<String>,
    pub is_failure_derived: bool,
}

impl TrackedTask {
    /// Create from a description and affected files
    pub fn new(
        id: TaskId,
        description: String,
        affected_files: Vec<String>,
        is_failure_derived: bool,
    ) -> Self {
        Self {
            id,
            description,
            affected_files,
            is_failure_derived,
        }
    }
}

/// Novelty Checker for rejecting duplicate tasks
pub struct NoveltyChecker {
    config: NoveltyCheckerConfig,
    /// Existing tasks being tracked for deduplication
    tracked_tasks: Vec<TrackedTask>,
}

impl NoveltyChecker {
    /// Create a new novelty checker with default config
    pub fn new() -> Self {
        Self {
            config: NoveltyCheckerConfig::default(),
            tracked_tasks: Vec::new(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(config: NoveltyCheckerConfig) -> Self {
        Self {
            config,
            tracked_tasks: Vec::new(),
        }
    }

    /// Check if a task is novel (not a duplicate)
    ///
    /// Returns a [`NoveltyCheckResult`] indicating whether the task is novel
    /// and providing details about similarity and file overlap.
    pub fn check(
        &self,
        description: &str,
        affected_files: &[String],
        is_failure_derived: bool,
    ) -> NoveltyCheckResult {
        if !self.config.enabled {
            return NoveltyCheckResult::novel(0.0, 0.0);
        }

        let mut max_similarity = 0.0f32;
        let mut max_file_overlap = 0.0f32;

        for existing in &self.tracked_tasks {
            // Check file overlap first (always checked, even for failure-derived)
            let file_overlap = self.compute_file_overlap(affected_files, &existing.affected_files);
            max_file_overlap = max_file_overlap.max(file_overlap);

            // Check if file overlap is significant
            if file_overlap >= self.config.file_overlap_threshold {
                debug!(
                    task_id = %existing.id,
                    file_overlap = %file_overlap,
                    "Significant file overlap detected"
                );
                return NoveltyCheckResult::duplicate(
                    existing.id,
                    format!(
                        "Duplicate task rejected: {:.1}% file overlap with existing task {}",
                        file_overlap * 100.0,
                        existing.id
                    ),
                    max_similarity,
                    file_overlap,
                );
            }

            // For failure-derived tasks, skip description similarity check
            // This allows multiple "Fix error X" tasks for different errors
            if is_failure_derived {
                continue;
            }

            // Check description similarity for non-failure-derived tasks
            let similarity = self.compute_similarity(description, &existing.description);
            max_similarity = max_similarity.max(similarity);

            // Check if similarity is high enough to be considered duplicate
            if similarity >= self.config.similarity_threshold {
                debug!(
                    task_id = %existing.id,
                    similarity = %similarity,
                    "High description similarity detected"
                );
                return NoveltyCheckResult::duplicate(
                    existing.id,
                    format!(
                        "Duplicate task rejected: {:.0}% similar to existing task {}",
                        similarity * 100.0,
                        existing.id
                    ),
                    similarity,
                    max_file_overlap,
                );
            }
        }

        if max_similarity > 0.0 || max_file_overlap > 0.0 {
            info!(
                max_similarity = %max_similarity,
                max_file_overlap = %max_file_overlap,
                "Task is novel but has some overlap with existing tasks"
            );
        }

        NoveltyCheckResult::novel(max_similarity, max_file_overlap)
    }

    /// Register an existing task for future novelty comparisons
    pub fn track_task(&mut self, task: TrackedTask) {
        info!(
            task_id = %task.id,
            description_len = task.description.len(),
            file_count = task.affected_files.len(),
            "Tracking task for novelty checking"
        );
        self.tracked_tasks.push(task);
    }

    /// Unregister a task (e.g., when task is completed or cancelled)
    pub fn untrack_task(&mut self, task_id: TaskId) {
        if let Some(pos) = self.tracked_tasks.iter().position(|t| t.id == task_id) {
            let removed = self.tracked_tasks.remove(pos);
            debug!(
                task_id = %removed.id,
                "Untracked task"
            );
        }
    }

    /// Get the number of tracked tasks
    pub fn tracked_count(&self) -> usize {
        self.tracked_tasks.len()
    }

    /// Clear all tracked tasks
    pub fn clear(&mut self) {
        self.tracked_tasks.clear();
        debug!("Cleared all tracked tasks");
    }

    /// Compute similarity between two descriptions using Levenshtein distance
    fn compute_similarity(&self, desc1: &str, desc2: &str) -> f32 {
        let distance = levenshtein_distance(desc1, desc2);
        let max_len = desc1.len().max(desc2.len());
        if max_len == 0 {
            return 0.0;
        }

        1.0 - (distance as f32 / max_len as f32)
    }

    /// Compute file overlap ratio between two sets of files
    fn compute_file_overlap(&self, files1: &[String], files2: &[String]) -> f32 {
        if files1.is_empty() || files2.is_empty() {
            return 0.0;
        }

        let set1: HashSet<&str> = files1.iter().map(|s| s.as_str()).collect();
        let set2: HashSet<&str> = files2.iter().map(|s| s.as_str()).collect();

        let intersection_count = set1.intersection(&set2).count();
        let min_len = files1.len().min(files2.len());

        if min_len == 0 {
            return 0.0;
        }

        intersection_count as f32 / min_len as f32
    }

    /// Update configuration
    pub fn set_config(&mut self, config: NoveltyCheckerConfig) {
        self.config = config;
        info!(
            enabled = self.config.enabled,
            similarity_threshold = self.config.similarity_threshold,
            file_overlap_threshold = self.config.file_overlap_threshold,
            "Novelty checker config updated"
        );
    }

    /// Get current configuration
    pub fn config(&self) -> &NoveltyCheckerConfig {
        &self.config
    }
}

impl Default for NoveltyChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Calculate Levenshtein distance between two strings
#[allow(clippy::needless_range_loop)]
pub fn levenshtein_distance(s1: &str, s2: &str) -> usize {
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

    // --- NoveltyChecker Tests ---

    #[test]
    fn test_novel_task_is_accepted() {
        let checker = NoveltyChecker::new();

        let result = checker.check("Implement feature X", &["src/x.rs".to_string()], false);

        assert!(result.is_novel);
        assert!(result.duplicate_of.is_none());
    }

    #[test]
    fn test_duplicate_by_description() {
        let mut checker = NoveltyChecker::new();

        // Track an existing task
        checker.track_task(TrackedTask::new(
            TaskId::new(),
            "Add user authentication to the login page".to_string(),
            vec!["src/auth.rs".to_string()],
            false,
        ));

        // New task with very similar description but different files
        // 50 chars vs 44 chars, difference is " now" (5 chars)
        // similarity = 1 - 5/50 = 0.9 = 90% > 85% threshold
        let result = checker.check(
            "Add user authentication to the login page now",
            &["src/other.rs".to_string()],
            false,
        );

        assert!(!result.is_novel);
        assert!(result.duplicate_of.is_some());
        assert!(result.rejection_reason.is_some());
        assert!(result
            .rejection_reason
            .unwrap()
            .contains("similar to existing task"));
    }

    #[test]
    fn test_duplicate_by_file_overlap() {
        let mut checker = NoveltyChecker::new();

        // Track an existing task with specific files
        checker.track_task(TrackedTask::new(
            TaskId::new(),
            "Implement login feature".to_string(),
            vec![
                "src/auth.rs".to_string(),
                "src/session.rs".to_string(),
                "src/user.rs".to_string(),
                "src/permissions.rs".to_string(),
                "src/login.rs".to_string(),
            ],
            false,
        ));

        // New task with 4 overlapping files out of 5 = 80% overlap
        // This meets the 80% threshold
        let result = checker.check(
            "Fix authentication bug",
            &[
                "src/auth.rs".to_string(),
                "src/session.rs".to_string(),
                "src/user.rs".to_string(),
                "src/permissions.rs".to_string(),
                "src/other.rs".to_string(),
            ],
            false,
        );

        // 4 overlapping files out of 5 in the smallest set = 80% overlap
        // This equals the 80% threshold
        assert!(!result.is_novel);
        assert!(result.duplicate_of.is_some());
        assert!(result.rejection_reason.unwrap().contains("file overlap"));
    }

    #[test]
    fn test_failure_derived_exempt_from_description_check() {
        let mut checker = NoveltyChecker::new();

        // Track an existing task
        checker.track_task(TrackedTask::new(
            TaskId::new(),
            "Fix type error in auth.ts".to_string(),
            vec!["src/auth.ts".to_string()],
            true, // is_failure_derived
        ));

        // New failure-derived task with similar description but different files
        // Should NOT be rejected based on description similarity
        let result = checker.check(
            "Fix type error in auth.ts line 42",
            &["src/other.rs".to_string()],
            true, // is_failure_derived
        );

        assert!(result.is_novel); // Should be accepted since it's failure-derived
    }

    #[test]
    fn test_failure_derived_still_checks_file_overlap() {
        let mut checker = NoveltyChecker::new();

        // Track an existing task with specific files
        checker.track_task(TrackedTask::new(
            TaskId::new(),
            "Fix type error in auth.ts".to_string(),
            vec![
                "src/auth.ts".to_string(),
                "src/session.rs".to_string(),
                "src/user.rs".to_string(),
                "src/permissions.rs".to_string(),
            ],
            true, // is_failure_derived
        ));

        // New failure-derived task with 3 overlapping files out of 4 = 75% overlap
        // This is below 80% threshold, so no rejection by file overlap alone
        let result_below = checker.check(
            "Fix different error in login.ts",
            &[
                "src/auth.ts".to_string(),
                "src/session.rs".to_string(),
                "src/user.rs".to_string(),
                "src/login.ts".to_string(),
            ],
            true, // is_failure_derived
        );

        // 75% overlap is below 80% threshold, so description check would matter
        // But descriptions are different enough, so it's novel
        assert!(result_below.is_novel);

        // Now test with 80% overlap - 4 files overlapping out of 4 = 100%
        let result_above = checker.check(
            "Fix different error in login.ts",
            &[
                "src/auth.ts".to_string(),
                "src/session.rs".to_string(),
                "src/user.rs".to_string(),
                "src/permissions.rs".to_string(),
            ],
            true, // is_failure_derived
        );

        // 100% file overlap - exceeds 80% threshold even for failure-derived
        assert!(!result_above.is_novel);
        assert!(result_above
            .rejection_reason
            .unwrap()
            .contains("file overlap"));
    }

    #[test]
    fn test_track_and_untrack_task() {
        let mut checker = NoveltyChecker::new();
        let task_id = TaskId::new();

        assert_eq!(checker.tracked_count(), 0);

        checker.track_task(TrackedTask::new(
            task_id,
            "Test task".to_string(),
            vec!["src/test.rs".to_string()],
            false,
        ));

        assert_eq!(checker.tracked_count(), 1);

        checker.untrack_task(task_id);

        assert_eq!(checker.tracked_count(), 0);
    }

    #[test]
    fn test_clear_tracked_tasks() {
        let mut checker = NoveltyChecker::new();

        checker.track_task(TrackedTask::new(
            TaskId::new(),
            "Task 1".to_string(),
            vec![],
            false,
        ));
        checker.track_task(TrackedTask::new(
            TaskId::new(),
            "Task 2".to_string(),
            vec![],
            false,
        ));

        assert_eq!(checker.tracked_count(), 2);

        checker.clear();

        assert_eq!(checker.tracked_count(), 0);
    }

    #[test]
    fn test_disabled_checker_accepts_all() {
        let mut config = NoveltyCheckerConfig::default();
        config.enabled = false;
        let _checker = NoveltyChecker::with_config(config.clone());

        let mut checker_with_task = NoveltyChecker::with_config(config);
        checker_with_task.track_task(TrackedTask::new(
            TaskId::new(),
            "Existing task".to_string(),
            vec!["src/auth.rs".to_string()],
            false,
        ));

        // Even with tracked task, should accept all when disabled
        let result = checker_with_task.check(
            "Exactly same task description",
            &["src/auth.rs".to_string()],
            false,
        );

        assert!(result.is_novel); // No rejection when disabled
    }

    #[test]
    fn test_config_update() {
        let mut checker = NoveltyChecker::new();

        assert_eq!(checker.config().similarity_threshold, 0.85);

        let mut new_config = NoveltyCheckerConfig::default();
        new_config.similarity_threshold = 0.95;
        checker.set_config(new_config);

        assert_eq!(checker.config().similarity_threshold, 0.95);
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

    #[test]
    fn test_levenshtein_similarity_conversion() {
        // Create a checker to test similarity computation
        let checker = NoveltyChecker::new();

        // 100% identical
        let sim = checker.compute_similarity("hello", "hello");
        assert!((sim - 1.0).abs() < 0.001);

        // 80% similar (1 char diff out of 5)
        let sim = checker.compute_similarity("hello", "hallo");
        assert!((sim - 0.8).abs() < 0.001);
    }

    // --- NoveltyCheckResult Tests ---

    #[test]
    fn test_novelty_check_result_novel() {
        let result = NoveltyCheckResult::novel(0.3, 0.2);

        assert!(result.is_novel);
        assert_eq!(result.max_similarity, 0.3);
        assert_eq!(result.max_file_overlap, 0.2);
        assert!(result.duplicate_of.is_none());
        assert!(result.rejection_reason.is_none());
    }

    #[test]
    fn test_novelty_check_result_duplicate() {
        let duplicate_id = Uuid::new_v4();
        let result =
            NoveltyCheckResult::duplicate(duplicate_id, "Too similar".to_string(), 0.9, 0.1);

        assert!(!result.is_novel);
        assert_eq!(result.duplicate_of, Some(duplicate_id));
        assert!(result.rejection_reason.is_some());
        assert_eq!(result.rejection_reason.unwrap(), "Too similar");
    }

    // --- TrackedTask Tests ---

    #[test]
    fn test_tracked_task_creation() {
        let task = TrackedTask::new(
            TaskId::new(),
            "Test description".to_string(),
            vec!["file1.rs".to_string(), "file2.rs".to_string()],
            false,
        );

        assert_eq!(task.description, "Test description");
        assert_eq!(task.affected_files.len(), 2);
        assert!(!task.is_failure_derived);
    }

    #[test]
    fn test_tracked_task_failure_derived() {
        let task = TrackedTask::new(TaskId::new(), "Fix error".to_string(), vec![], true);

        assert!(task.is_failure_derived);
    }

    // --- Config Tests ---

    #[test]
    fn test_default_config() {
        let config = NoveltyCheckerConfig::default();

        assert!(config.enabled);
        assert_eq!(config.similarity_threshold, 0.85);
        assert_eq!(config.file_overlap_threshold, 0.8); // 80% file overlap threshold
        assert_eq!(config.max_levenshtein_distance, 30);
    }

    #[test]
    fn test_custom_config() {
        let config = NoveltyCheckerConfig {
            enabled: true,
            similarity_threshold: 0.9,
            file_overlap_threshold: 0.6,
            max_levenshtein_distance: 20,
        };

        assert_eq!(config.similarity_threshold, 0.9);
        assert_eq!(config.file_overlap_threshold, 0.6);
    }

    // --- Edge Cases ---

    #[test]
    fn test_empty_description() {
        let checker = NoveltyChecker::new();

        let result = checker.check("", &["file.rs".to_string()], false);

        assert!(result.is_novel); // Empty shouldn't be considered duplicate
    }

    #[test]
    fn test_empty_files() {
        let checker = NoveltyChecker::new();

        let result = checker.check("Some description", &[], false);

        assert!(result.is_novel); // No files to compare
    }

    #[test]
    fn test_no_tracked_tasks() {
        let checker = NoveltyChecker::new();

        let result = checker.check("Any description", &["any_file.rs".to_string()], false);

        assert!(result.is_novel);
    }

    #[test]
    fn test_slightly_similar_not_duplicate() {
        let mut checker = NoveltyChecker::new();

        checker.track_task(TrackedTask::new(
            TaskId::new(),
            "Implement login feature".to_string(),
            vec![],
            false,
        ));

        // Very different description
        let result = checker.check(
            "Fix bug in payment module",
            &["src/payment.rs".to_string()],
            false,
        );

        assert!(result.is_novel);
    }
}
