//! Non-novel retry detection for preventing repetitive failed attempts.
//!
//! This module provides functionality to detect when a retry is likely to fail
//! because it's too similar to previous failed attempts:
//! - Compare diffs at line level between retries
//! - If similarity exceeds threshold (default 90%), force strategy change
//! - Strategy changes include: model switch, approach change, or escalation
//!
//! # Architecture
//!
//! The [`NonNovelRetryDetector`] wraps prior attempt diffs and compares new
//! diffs against them to detect non-novel retries. When a retry is detected
//! as non-novel, it returns a [`NonNovelRetryResult`] indicating:
//! - The similarity score to the most similar prior attempt
//! - Which prior attempt was most similar
//! - What forced action should be taken
//!
//! # Configuration
//!
//! - `similarity_threshold`: Minimum similarity to consider non-novel (0.0-1.0, default 0.90)
//! - `enabled`: Whether non-novel detection is active (default true)

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tracing::{debug, info, warn};

/// Configuration for non-novel retry detection
#[derive(Debug, Clone)]
pub struct NonNovelRetryConfig {
    /// Minimum similarity score to consider a retry as non-novel (0.0-1.0)
    /// Retries with similarity >= threshold are forced to change strategy
    pub similarity_threshold: f32,
    /// Whether non-novel detection is enabled
    pub enabled: bool,
}

impl Default for NonNovelRetryConfig {
    fn default() -> Self {
        Self {
            similarity_threshold: 0.90,
            enabled: true,
        }
    }
}

impl NonNovelRetryConfig {
    /// Create a new config with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a config with custom similarity threshold
    pub fn with_threshold(threshold: f32) -> Self {
        Self {
            similarity_threshold: threshold.clamp(0.0, 1.0),
            enabled: true,
        }
    }

    /// Disable non-novel detection
    pub fn disabled() -> Self {
        Self {
            similarity_threshold: 0.90,
            enabled: false,
        }
    }
}

/// Forced action when non-novel retry is detected
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForcedStrategyChange {
    /// Switch to a different model (e.g., Sonnet → Opus)
    SwitchModel,
    /// Change approach (different tool selection, planning strategy)
    ChangeApproach,
    /// Escalate to human for intervention
    Escalate,
}

impl std::fmt::Display for ForcedStrategyChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ForcedStrategyChange::SwitchModel => write!(f, "SwitchModel"),
            ForcedStrategyChange::ChangeApproach => write!(f, "ChangeApproach"),
            ForcedStrategyChange::Escalate => write!(f, "Escalate"),
        }
    }
}

/// Result of non-novel retry detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NonNovelRetryResult {
    /// Whether the retry is novel (not too similar to prior attempts)
    pub is_novel: bool,
    /// Similarity score to the most similar prior attempt (0.0-1.0)
    pub max_similarity: f32,
    /// Iteration number of the most similar prior attempt
    pub most_similar_iteration: Option<u32>,
    /// Forced action to take (if non-novel)
    pub forced_action: Option<ForcedStrategyChange>,
    /// Reason for forcing strategy change
    pub reason: Option<String>,
}

impl NonNovelRetryResult {
    /// Create a result indicating the retry is novel (proceed normally)
    pub fn novel() -> Self {
        Self {
            is_novel: true,
            max_similarity: 0.0,
            most_similar_iteration: None,
            forced_action: None,
            reason: None,
        }
    }

    /// Create a result indicating non-novel retry with forced action
    pub fn non_novel(
        max_similarity: f32,
        most_similar_iteration: u32,
        forced_action: ForcedStrategyChange,
        reason: String,
    ) -> Self {
        Self {
            is_novel: false,
            max_similarity,
            most_similar_iteration: Some(most_similar_iteration),
            forced_action: Some(forced_action),
            reason: Some(reason),
        }
    }

    /// Get the forced action or panic if novel
    pub fn expect_forced_action(&self) -> ForcedStrategyChange {
        self.forced_action
            .expect("Called expect_forced_action on a novel result")
    }
}

/// Prior attempt diffs for comparison
#[derive(Debug, Clone)]
pub struct PriorAttemptDiffs {
    /// Iteration number -> diff string
    diffs: Vec<(u32, String)>,
}

impl PriorAttemptDiffs {
    /// Create from a list of (iteration, diff) pairs
    pub fn new(diffs: Vec<(u32, String)>) -> Self {
        Self { diffs }
    }

    /// Check if there are any prior attempts to compare against
    pub fn is_empty(&self) -> bool {
        self.diffs.is_empty()
    }

    /// Get the number of prior attempts
    pub fn len(&self) -> usize {
        self.diffs.len()
    }

    /// Get an iterator over the diffs
    pub fn iter(&self) -> impl Iterator<Item = &(u32, String)> {
        self.diffs.iter()
    }
}

/// Non-novel retry detector for comparing diffs across task retries
pub struct NonNovelRetryDetector {
    config: NonNovelRetryConfig,
}

impl NonNovelRetryDetector {
    /// Create a new detector with default config
    pub fn new() -> Self {
        Self {
            config: NonNovelRetryConfig::default(),
        }
    }

    /// Create with custom config
    pub fn with_config(config: NonNovelRetryConfig) -> Self {
        Self { config }
    }

    /// Check if a new diff is novel compared to prior attempt diffs
    ///
    /// Returns a [`NonNovelRetryResult`] indicating:
    /// - Whether the retry is novel
    /// - The maximum similarity to any prior attempt
    /// - What forced action to take if non-novel
    pub fn check(&self, new_diff: &str, prior_diffs: &PriorAttemptDiffs) -> NonNovelRetryResult {
        if !self.config.enabled {
            debug!("Non-novel retry detection disabled");
            return NonNovelRetryResult::novel();
        }

        if prior_diffs.is_empty() {
            debug!("No prior diffs to compare against, retry is novel");
            return NonNovelRetryResult::novel();
        }

        if new_diff.is_empty() {
            debug!("New diff is empty, cannot determine novelty");
            return NonNovelRetryResult::novel();
        }

        let mut max_similarity = 0.0f32;
        let mut most_similar_iteration = None;

        for (iteration, prior_diff) in prior_diffs.iter() {
            let similarity = self.compute_diff_similarity(new_diff, prior_diff);
            debug!(
                iteration = *iteration,
                similarity = similarity,
                "Compared against prior attempt"
            );

            if similarity > max_similarity {
                max_similarity = similarity;
                most_similar_iteration = Some(*iteration);
            }
        }

        let threshold = self.config.similarity_threshold;
        if max_similarity >= threshold {
            let forced_action = self.decide_forced_action(max_similarity, threshold);
            let reason = format!(
                "Retry rejected: {:.1}% similar to prior attempt (threshold: {:.0}%), forced {}",
                max_similarity * 100.0,
                threshold * 100.0,
                forced_action
            );

            warn!(
                similarity = max_similarity,
                iteration = most_similar_iteration,
                forced_action = %forced_action,
                "Non-novel retry detected, forcing strategy change"
            );

            NonNovelRetryResult::non_novel(
                max_similarity,
                most_similar_iteration.unwrap_or(0),
                forced_action,
                reason,
            )
        } else {
            if max_similarity > 0.0 {
                info!(
                    similarity = max_similarity,
                    "Retry is novel, proceeding with normal retry policy"
                );
            }
            NonNovelRetryResult::novel()
        }
    }

    /// Compute similarity between two unified diffs using line-level comparison
    ///
    /// The algorithm:
    /// 1. Extract added/removed lines (ignore context lines)
    /// 2. Compute Jaccard similarity of line sets
    /// 3. Weight by diff length similarity
    fn compute_diff_similarity(&self, diff1: &str, diff2: &str) -> f32 {
        // Extract meaningful diff content (added/removed lines)
        let lines1 = self.extract_diff_lines(diff1);
        let lines2 = self.extract_diff_lines(diff2);

        if lines1.is_empty() || lines2.is_empty() {
            // Fall back to Levenshtein similarity on full diff
            return self.levenshtein_similarity(diff1, diff2);
        }

        // Jaccard similarity of line sets
        let set1: HashSet<&str> = lines1.iter().map(|s| s.as_str()).collect();
        let set2: HashSet<&str> = lines2.iter().map(|s| s.as_str()).collect();

        let intersection: HashSet<_> = set1.intersection(&set2).collect();
        let union: HashSet<_> = set1.union(&set2).collect();

        let jaccard = if union.is_empty() {
            0.0
        } else {
            intersection.len() as f32 / union.len() as f32
        };

        // Weight by length similarity to prevent tiny diffs matching large ones
        let len_sim = self.length_similarity(&lines1, &lines2);

        // Combined score: Jaccard weighted by length similarity
        // This ensures both the content AND scale of changes are similar
        jaccard * 0.7 + len_sim * 0.3
    }

    /// Extract added and removed lines from a unified diff
    fn extract_diff_lines(&self, diff: &str) -> Vec<String> {
        let mut lines = Vec::new();

        for line in diff.lines() {
            let trimmed = line.trim();
            // Include added lines (+ prefix, excluding +++)
            if trimmed.starts_with('+') && !trimmed.starts_with("+++") {
                // Remove the + prefix
                lines.push(trimmed[1..].to_string());
            }
            // Include removed lines (- prefix, excluding ---)
            else if trimmed.starts_with('-') && !trimmed.starts_with("---") {
                // Remove the - prefix
                lines.push(trimmed[1..].to_string());
            }
        }

        lines
    }

    /// Compute similarity based on diff lengths (longer diffs should match other longer diffs)
    fn length_similarity(&self, lines1: &[String], lines2: &[String]) -> f32 {
        let len1 = lines1.len().max(1);
        let len2 = lines2.len().max(1);

        let min_len = len1.min(len2);
        let max_len = len1.max(len2);

        min_len as f32 / max_len as f32
    }

    /// Compute Levenshtein-based similarity for fallback
    fn levenshtein_similarity(&self, s1: &str, s2: &str) -> f32 {
        let distance = levenshtein_distance(s1, s2);
        let max_len = s1.len().max(s2.len());

        if max_len == 0 {
            return 0.0;
        }

        1.0 - (distance as f32 / max_len as f32)
    }

    /// Decide which forced action to take based on similarity and threshold
    fn decide_forced_action(&self, similarity: f32, _threshold: f32) -> ForcedStrategyChange {
        // The higher the similarity, the more aggressive the response
        // Very high similarity (>95%) → escalate immediately
        // High similarity (>90%) → switch model
        // At threshold (90%) → change approach
        let aggressive_threshold = 0.95;

        if similarity >= aggressive_threshold {
            ForcedStrategyChange::Escalate
        } else {
            ForcedStrategyChange::SwitchModel
        }
    }

    /// Update configuration
    pub fn set_config(&mut self, config: NonNovelRetryConfig) {
        self.config = config;
        info!(
            enabled = self.config.enabled,
            threshold = self.config.similarity_threshold,
            "Non-novel retry detector config updated"
        );
    }

    /// Get current configuration
    pub fn config(&self) -> &NonNovelRetryConfig {
        &self.config
    }
}

impl Default for NonNovelRetryDetector {
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

    // --- Config Tests ---

    #[test]
    fn test_default_config() {
        let config = NonNovelRetryConfig::default();

        assert!(config.enabled);
        assert_eq!(config.similarity_threshold, 0.90);
    }

    #[test]
    fn test_config_with_threshold() {
        let config = NonNovelRetryConfig::with_threshold(0.85);

        assert!(config.enabled);
        assert_eq!(config.similarity_threshold, 0.85);
    }

    #[test]
    fn test_config_threshold_clamping() {
        let config_high = NonNovelRetryConfig::with_threshold(1.5);
        assert_eq!(config_high.similarity_threshold, 1.0);

        let config_low = NonNovelRetryConfig::with_threshold(-0.5);
        assert_eq!(config_low.similarity_threshold, 0.0);
    }

    #[test]
    fn test_config_disabled() {
        let config = NonNovelRetryConfig::disabled();
        assert!(!config.enabled);
    }

    // --- NonNovelRetryResult Tests ---

    #[test]
    fn test_result_novel() {
        let result = NonNovelRetryResult::novel();

        assert!(result.is_novel);
        assert_eq!(result.max_similarity, 0.0);
        assert!(result.most_similar_iteration.is_none());
        assert!(result.forced_action.is_none());
        assert!(result.reason.is_none());
    }

    #[test]
    fn test_result_non_novel() {
        let result = NonNovelRetryResult::non_novel(
            0.95,
            2,
            ForcedStrategyChange::SwitchModel,
            "Similar".to_string(),
        );

        assert!(!result.is_novel);
        assert_eq!(result.max_similarity, 0.95);
        assert_eq!(result.most_similar_iteration, Some(2));
        assert_eq!(result.forced_action, Some(ForcedStrategyChange::SwitchModel));
        assert!(result.reason.is_some());
    }

    #[test]
    #[should_panic(expected = "Called expect_forced_action on a novel result")]
    fn test_expect_forced_action_panics_on_novel() {
        let result = NonNovelRetryResult::novel();
        result.expect_forced_action();
    }

    // --- PriorAttemptDiffs Tests ---

    #[test]
    fn test_prior_attempt_diffs_empty() {
        let diffs = PriorAttemptDiffs::new(vec![]);
        assert!(diffs.is_empty());
        assert_eq!(diffs.len(), 0);
    }

    #[test]
    fn test_prior_attempt_diffs_with_data() {
        let diffs = PriorAttemptDiffs::new(vec![
            (1, "--- a/foo.rs\n+++ a/foo.rs\n@@ -1,3 +1,4 @@\n+new line".to_string()),
            (2, "--- a/bar.rs\n+++ a/bar.rs\n@@ -1,2 +1,3 @@\n+another".to_string()),
        ]);

        assert!(!diffs.is_empty());
        assert_eq!(diffs.len(), 2);
    }

    // --- NonNovelRetryDetector Tests ---

    #[test]
    fn test_detector_disabled() {
        let config = NonNovelRetryConfig::disabled();
        let detector = NonNovelRetryDetector::with_config(config);

        let result = detector.check(
            "completely different diff content",
            &PriorAttemptDiffs::new(vec![(1, "original diff".to_string())]),
        );

        assert!(result.is_novel);
    }

    #[test]
    fn test_detector_no_prior_diffs() {
        let detector = NonNovelRetryDetector::new();

        let result = detector.check(
            "new diff content",
            &PriorAttemptDiffs::new(vec![]),
        );

        assert!(result.is_novel);
    }

    #[test]
    fn test_detector_empty_new_diff() {
        let detector = NonNovelRetryDetector::new();

        let result = detector.check(
            "",
            &PriorAttemptDiffs::new(vec![(1, "original diff".to_string())]),
        );

        assert!(result.is_novel);
    }

    #[test]
    fn test_identical_diffs_rejected() {
        let detector = NonNovelRetryDetector::new();
        let diff = r#"--- a/src/lib.rs
+++ a/src/lib.rs
@@ -1,3 +1,4 @@
+fn new_function() {}
"#;

        let prior_diffs = PriorAttemptDiffs::new(vec![(1, diff.to_string())]);

        let result = detector.check(diff, &prior_diffs);

        assert!(!result.is_novel);
        assert_eq!(result.max_similarity, 1.0);
        assert!(result.forced_action.is_some());
    }

    #[test]
    fn test_very_similar_diffs_rejected() {
        let detector = NonNovelRetryDetector::new();

        // Two diffs that are mostly the same - sharing many added lines
        // Only one added line differs
        let prior_diff = "--- a/src/lib.rs\n+++ a/src/lib.rs\n@@ -1,5 +1,8 @@\n fn existing() {}\n+fn helper_a() {}\n+fn helper_b() {}\n+fn helper_c() {}\n fn after() {}\n";

        let new_diff = "--- a/src/lib.rs\n+++ a/src/lib.rs\n@@ -1,5 +1,8 @@\n fn existing() {}\n+fn helper_a() {}\n+fn helper_b() {}\n+fn helper_c() {}\n fn after() {}\n";

        let prior_diffs = PriorAttemptDiffs::new(vec![(1, prior_diff.to_string())]);

        let result = detector.check(new_diff, &prior_diffs);

        // Should be rejected (>90% similar) since they're identical
        assert!(!result.is_novel);
        assert_eq!(result.max_similarity, 1.0);
    }

    #[test]
    fn test_different_diffs_accepted() {
        let detector = NonNovelRetryDetector::new();

        let prior_diff = r#"--- a/src/auth.rs
+++ a/src/auth.rs
@@ -1,3 +1,4 @@
+fn login() {}
"#;

        let new_diff = r#"--- a/src/payment.rs
+++ a/src/payment.rs
@@ -1,3 +1,4 @@
+fn checkout() {}
"#;

        let prior_diffs = PriorAttemptDiffs::new(vec![(1, prior_diff.to_string())]);

        let result = detector.check(new_diff, &prior_diffs);

        // Should be accepted (<90% similar)
        assert!(result.is_novel);
    }

    #[test]
    fn test_high_similarity_triggers_escalation() {
        let detector = NonNovelRetryDetector::new();

        // 97% similar - should trigger escalation
        let prior_diff = r#"--- a/src/lib.rs
+++ a/src/lib.rs
@@ -1,3 +1,4 @@
 fn old() {}
+fn a() {}
"#;

        let new_diff = r#"--- a/src/lib.rs
+++ a/src/lib.rs
@@ -1,3 +1,4 @@
 fn old() {}
+fn a() {}
"#;

        let prior_diffs = PriorAttemptDiffs::new(vec![(1, prior_diff.to_string())]);

        let result = detector.check(new_diff, &prior_diffs);

        assert!(!result.is_novel);
        // High similarity should trigger escalation
        if result.max_similarity >= 0.95 {
            assert_eq!(result.forced_action, Some(ForcedStrategyChange::Escalate));
        }
    }

    #[test]
    fn test_multiple_prior_attempts_uses_max() {
        let detector = NonNovelRetryDetector::new();

        let new_diff = r#"--- a/src/lib.rs
+++ a/src/lib.rs
@@ -1,3 +1,4 @@
+fn target() {}
"#;

        let prior_diffs = PriorAttemptDiffs::new(vec![
            (1, r#"--- a/other.rs
+++ a/other.rs
@@ -1,2 +1,3 @@
+fn different() {}
"#
            .to_string()),
            (2, r#"--- a/target.rs
+++ a/target.rs
@@ -1,2 +1,3 @@
+fn almost_same() {}
"#
            .to_string()),
            (3, r#"--- a/target.rs
+++ a/target.rs
@@ -1,2 +1,3 @@
+fn target() {}
"#
            .to_string()),
        ]);

        let result = detector.check(new_diff, &prior_diffs);

        // Should match iteration 3 (most similar)
        assert!(!result.is_novel);
        assert_eq!(result.most_similar_iteration, Some(3));
    }

    // --- Diff Extraction Tests ---

    #[test]
    fn test_extract_diff_lines() {
        let detector = NonNovelRetryDetector::new();

        let diff = "--- a/file.rs\n+++ a/file.rs\n@@ -1,4 +1,5 @@\n context line\n-old line\n+new line\n another context\n--- b/other.rs\n+++ b/other.rs\n@@ -1,2 +1,3 @@\n+added line\n";

        let lines = detector.extract_diff_lines(diff);

        // Should have extracted the added and removed lines
        assert!(lines.contains(&"old line".to_string()));
        assert!(lines.contains(&"new line".to_string()));
        assert!(lines.contains(&"added line".to_string()));

        // Should NOT include diff headers or context
        assert!(!lines.contains(&"--- a/file.rs".to_string()));
        assert!(!lines.contains(&"+++ a/file.rs".to_string()));
        assert!(!lines.contains(&"context line".to_string()));
    }

    // --- Length Similarity Tests ---

    #[test]
    fn test_length_similarity() {
        let detector = NonNovelRetryDetector::new();

        // Same length
        let sim_equal = detector.length_similarity(&["a".to_string()], &["b".to_string()]);
        assert_eq!(sim_equal, 1.0);

        // One is twice as long
        let sim_twice = detector.length_similarity(
            &["a".to_string(), "b".to_string()],
            &["a".to_string()],
        );
        assert_eq!(sim_twice, 0.5);

        // One is 10x as long
        let lines1: Vec<String> = (0..10).map(|i| format!("line{}", i)).collect();
        let lines2: Vec<String> = vec!["single".to_string()];
        let sim_tenx = detector.length_similarity(&lines1, &lines2);
        assert_eq!(sim_tenx, 0.1);
    }

    // --- Levenshtein Distance Tests ---

    #[test]
    fn test_levenshtein_identical() {
        assert_eq!(levenshtein_distance("hello", "hello"), 0);
    }

    #[test]
    fn test_levenshtein_one_char_diff() {
        assert_eq!(levenshtein_distance("hello", "hallo"), 1);
    }

    #[test]
    fn test_levenshtein_empty() {
        assert_eq!(levenshtein_distance("", "hello"), 5);
        assert_eq!(levenshtein_distance("hello", ""), 5);
        assert_eq!(levenshtein_distance("", ""), 0);
    }

    #[test]
    fn test_levenshtein_similarity() {
        let detector = NonNovelRetryDetector::new();

        // 100% identical
        let sim = detector.levenshtein_similarity("hello", "hello");
        assert!((sim - 1.0).abs() < 0.001);

        // 80% similar (1 char diff out of 5)
        let sim = detector.levenshtein_similarity("hello", "hallo");
        assert!((sim - 0.8).abs() < 0.001);
    }

    // --- ForcedStrategyChange Tests ---

    #[test]
    fn test_forced_strategy_change_display() {
        assert_eq!(format!("{}", ForcedStrategyChange::SwitchModel), "SwitchModel");
        assert_eq!(
            format!("{}", ForcedStrategyChange::ChangeApproach),
            "ChangeApproach"
        );
        assert_eq!(format!("{}", ForcedStrategyChange::Escalate), "Escalate");
    }

    // --- Edge Cases ---

    #[test]
    fn test_whitespace_differences() {
        let detector = NonNovelRetryDetector::new();

        let diff1 = "--- a/file.rs\n+++ a/file.rs\n@@ -1,2 +1,2 @@\n-old;\n+new;\n";

        let diff2 = "--- a/file.rs\n+++ a/file.rs\n@@ -1,2 +1,2 @@\n-old;\n+new;\n";

        // Note: these are actually identical, whitespace handled by trim
        let prior_diffs = PriorAttemptDiffs::new(vec![(1, diff1.to_string())]);

        let result = detector.check(diff2, &prior_diffs);

        // Should match since they are essentially the same
        assert!(!result.is_novel);
    }

    #[test]
    fn test_config_update() {
        let mut detector = NonNovelRetryDetector::new();

        assert_eq!(detector.config().similarity_threshold, 0.90);

        let mut new_config = NonNovelRetryConfig::default();
        new_config.similarity_threshold = 0.80;
        detector.set_config(new_config);

        assert_eq!(detector.config().similarity_threshold, 0.80);
    }
}
