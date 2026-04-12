//! Value Scorer - scores tasks 1-5 on spec alignment and blocking impact for prioritization.
//!
//! This module provides functionality to:
//! - Score tasks 1-5 on spec alignment (how well task matches the spec)
//! - Score tasks 1-5 on blocking impact (how much task blocks other work)
//! - Combine scores into a single priority score for task scheduling
//!
//! # Scoring Scales (1-5)
//!
//! ## Spec Alignment Score
//! - 5: Directly mentioned in spec as required
//! - 4: Implements a clear spec requirement
//! - 3: Supports a spec requirement (enabling/supplementary)
//! - 2: Tangentially related to spec
//! - 1: Not mentioned in spec at all
//!
//! ## Blocking Impact Score
//! - 5: Other tasks blocked until this completes (critical path)
//! - 4: Multiple tasks blocked
//! - 3: Some tasks blocked
//! - 2: Few tasks blocked (indirect)
//! - 1: No tasks blocked
//!
//! # Combined Score
//!
//! The combined priority score is a weighted average:
//! `priority = spec_alignment * spec_weight + blocking_impact * blocking_weight`
//!
//! Default weights: spec=0.6, blocking=0.4

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tracing::debug;
use uuid::Uuid;

/// Spec alignment level (1-5)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecAlignmentScore(u8);

impl SpecAlignmentScore {
    /// Create a new spec alignment score (clamps to 1-5)
    pub fn new(score: u8) -> Self {
        Self(score.clamp(1, 5))
    }

    /// Score 1: Not mentioned in spec at all
    pub fn not_mentioned() -> Self {
        Self(1)
    }

    /// Score 2: Tangentially related to spec
    pub fn tangential() -> Self {
        Self(2)
    }

    /// Score 3: Supports a spec requirement (enabling/supplementary)
    pub fn supportive() -> Self {
        Self(3)
    }

    /// Score 4: Implements a clear spec requirement
    pub fn implements() -> Self {
        Self(4)
    }

    /// Score 5: Directly mentioned in spec as required
    pub fn required() -> Self {
        Self(5)
    }

    /// Get the raw score value
    pub fn value(&self) -> u8 {
        self.0
    }

    /// Get as f32 for calculations
    pub fn as_f32(&self) -> f32 {
        self.0 as f32
    }
}

impl Default for SpecAlignmentScore {
    fn default() -> Self {
        Self::not_mentioned()
    }
}

impl std::fmt::Display for SpecAlignmentScore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Blocking impact level (1-5)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockingImpactScore(u8);

impl BlockingImpactScore {
    /// Create a new blocking impact score (clamps to 1-5)
    pub fn new(score: u8) -> Self {
        Self(score.clamp(1, 5))
    }

    /// Score 1: No tasks blocked
    pub fn none() -> Self {
        Self(1)
    }

    /// Score 2: Few tasks blocked (indirect)
    pub fn few() -> Self {
        Self(2)
    }

    /// Score 3: Some tasks blocked
    pub fn some() -> Self {
        Self(3)
    }

    /// Score 4: Multiple tasks blocked
    pub fn multiple() -> Self {
        Self(4)
    }

    /// Score 5: Other tasks blocked until this completes (critical path)
    pub fn critical_path() -> Self {
        Self(5)
    }

    /// Get the raw score value
    pub fn value(&self) -> u8 {
        self.0
    }

    /// Get as f32 for calculations
    pub fn as_f32(&self) -> f32 {
        self.0 as f32
    }
}

impl Default for BlockingImpactScore {
    fn default() -> Self {
        Self::none()
    }
}

impl std::fmt::Display for BlockingImpactScore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A task dependency relationship for blocking analysis
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TaskDependency {
    /// The blocking task ID
    pub blocker: Uuid,
    /// The blocked task ID
    pub blocked: Uuid,
}

/// Result of scoring a task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskScore {
    /// Task ID
    pub task_id: Uuid,
    /// Spec alignment score (1-5)
    pub spec_alignment: SpecAlignmentScore,
    /// Blocking impact score (1-5)
    pub blocking_impact: BlockingImpactScore,
    /// Combined priority score (1-5, weighted average)
    pub priority_score: f32,
    /// Which other tasks this task blocks
    pub blocks: Vec<Uuid>,
    /// Which tasks this task depends on
    pub depends_on: Vec<Uuid>,
}

impl TaskScore {
    /// Create a new task score
    pub fn new(
        task_id: Uuid,
        spec_alignment: SpecAlignmentScore,
        blocking_impact: BlockingImpactScore,
        blocks: Vec<Uuid>,
        depends_on: Vec<Uuid>,
    ) -> Self {
        let priority_score = Self::compute_priority(spec_alignment, blocking_impact);
        Self {
            task_id,
            spec_alignment,
            blocking_impact,
            priority_score,
            blocks,
            depends_on,
        }
    }

    /// Compute the combined priority score
    fn compute_priority(
        spec_alignment: SpecAlignmentScore,
        blocking_impact: BlockingImpactScore,
    ) -> f32 {
        // Default weights: spec=0.6, blocking=0.4
        let spec_weight = 0.6;
        let blocking_weight = 0.4;

        spec_alignment.as_f32() * spec_weight + blocking_impact.as_f32() * blocking_weight
    }

    /// Get the priority as an integer (rounded)
    pub fn priority_rounded(&self) -> u8 {
        (self.priority_score + 0.5) as u8
    }
}

/// Configuration for value scoring
#[derive(Debug, Clone)]
pub struct ValueScorerConfig {
    /// Weight for spec alignment in combined score (0.0-1.0)
    pub spec_weight: f32,
    /// Weight for blocking impact in combined score (0.0-1.0)
    pub blocking_weight: f32,
}

impl Default for ValueScorerConfig {
    fn default() -> Self {
        Self {
            spec_weight: 0.6,
            blocking_weight: 0.4,
        }
    }
}

/// Value Scorer for scoring tasks on spec alignment and blocking impact
pub struct ValueScorer {
    config: ValueScorerConfig,
    /// Map of task ID to spec requirement IDs it fulfills
    spec_requirements: HashMap<Uuid, Vec<Uuid>>,
    /// Map of spec requirement ID to task IDs that implement it
    requirement_implementers: HashMap<Uuid, Vec<Uuid>>,
    /// Known spec requirements
    known_requirements: HashSet<Uuid>,
    /// Dependency graph (blocker -> blocked)
    dependencies: HashMap<Uuid, Vec<Uuid>>,
}

impl ValueScorer {
    /// Create a new value scorer with default config
    pub fn new() -> Self {
        Self::with_config(ValueScorerConfig::default())
    }

    /// Create with custom configuration
    pub fn with_config(config: ValueScorerConfig) -> Self {
        let spec_weight = config.spec_weight;
        let blocking_weight = config.blocking_weight;
        // Ensure weights sum to 1.0
        let total = spec_weight + blocking_weight;
        assert!(
            (total - 1.0).abs() < 0.001,
            "Weights must sum to 1.0, got {}",
            total
        );

        Self {
            config,
            spec_requirements: HashMap::new(),
            requirement_implementers: HashMap::new(),
            known_requirements: HashSet::new(),
            dependencies: HashMap::new(),
        }
    }

    /// Register a task as implementing a spec requirement
    pub fn register_spec_link(&mut self, task_id: Uuid, requirement_id: Uuid) {
        debug!(
            task_id = %task_id,
            requirement_id = %requirement_id,
            "Registering spec link"
        );
        self.spec_requirements
            .entry(task_id)
            .or_default()
            .push(requirement_id);
        self.requirement_implementers
            .entry(requirement_id)
            .or_default()
            .push(task_id);
        self.known_requirements.insert(requirement_id);
    }

    /// Register a dependency (blocker -> blocked)
    pub fn register_dependency(&mut self, blocker: Uuid, blocked: Uuid) {
        debug!(
            blocker = %blocker,
            blocked = %blocked,
            "Registering dependency"
        );
        self.dependencies.entry(blocker).or_default().push(blocked);
    }

    /// Score a task based on spec alignment and blocking impact
    pub fn score_task(&self, task_id: Uuid) -> TaskScore {
        let spec_alignment = self.compute_spec_alignment(task_id);
        let blocking_impact = self.compute_blocking_impact(task_id);
        let blocks = self.get_blocked_tasks(task_id);
        let depends_on = self.get_blocking_tasks(task_id);

        TaskScore::new(task_id, spec_alignment, blocking_impact, blocks, depends_on)
    }

    /// Score multiple tasks
    pub fn score_tasks(&self, task_ids: &[Uuid]) -> Vec<TaskScore> {
        task_ids.iter().map(|id| self.score_task(*id)).collect()
    }

    /// Compute spec alignment score (1-5)
    fn compute_spec_alignment(&self, task_id: Uuid) -> SpecAlignmentScore {
        // Check if task is directly linked to a spec requirement
        if let Some(requirements) = self.spec_requirements.get(&task_id) {
            if !requirements.is_empty() {
                // Directly implements spec requirement(s)
                return SpecAlignmentScore::implements();
            }
        }

        // Check if this task enables other tasks that implement spec requirements
        let enables_count = self.count_enables_spec_fulfilling(task_id);
        if enables_count > 0 {
            return SpecAlignmentScore::supportive();
        }

        // Check if task is mentioned indirectly via blocked tasks
        let blocks_spec = self.blocks_tasks_with_spec_links(task_id);
        if blocks_spec {
            return SpecAlignmentScore::tangential();
        }

        // No spec relation
        SpecAlignmentScore::not_mentioned()
    }

    /// Count how many tasks this task enables that fulfill spec requirements
    fn count_enables_spec_fulfilling(&self, task_id: Uuid) -> usize {
        let blocked_tasks = self.dependencies.get(&task_id);

        let mut count = 0;
        if let Some(blocked) = blocked_tasks {
            for blocked_id in blocked {
                if let Some(reqs) = self.spec_requirements.get(blocked_id) {
                    if !reqs.is_empty() {
                        count += 1;
                    }
                }
            }
        }

        count
    }

    /// Check if this task blocks tasks that have spec links
    fn blocks_tasks_with_spec_links(&self, task_id: Uuid) -> bool {
        if let Some(blocked) = self.dependencies.get(&task_id) {
            for blocked_id in blocked {
                if self.spec_requirements.contains_key(blocked_id) {
                    return true;
                }
            }
        }
        false
    }

    /// Compute blocking impact score (1-5)
    fn compute_blocking_impact(&self, task_id: Uuid) -> BlockingImpactScore {
        let blocked_tasks = self.dependencies.get(&task_id);
        let count = blocked_tasks.map(|v| v.len()).unwrap_or(0);

        match count {
            0 => BlockingImpactScore::none(),
            1 => BlockingImpactScore::few(),
            2..=3 => BlockingImpactScore::some(),
            4..=6 => BlockingImpactScore::multiple(),
            _ => BlockingImpactScore::critical_path(),
        }
    }

    /// Get tasks that this task blocks
    pub fn get_blocked_tasks(&self, task_id: Uuid) -> Vec<Uuid> {
        self.dependencies.get(&task_id).cloned().unwrap_or_default()
    }

    /// Get tasks that this task depends on (tasks that block it)
    pub fn get_blocking_tasks(&self, task_id: Uuid) -> Vec<Uuid> {
        let mut blocking = Vec::new();
        for (blocker, blocked_list) in &self.dependencies {
            if blocked_list.contains(&task_id) {
                blocking.push(*blocker);
            }
        }
        blocking
    }

    /// Get the number of tasks blocked by a task
    pub fn blocked_count(&self, task_id: Uuid) -> usize {
        self.dependencies
            .get(&task_id)
            .map(|v| v.len())
            .unwrap_or(0)
    }

    /// Get the number of tasks a task depends on
    pub fn blocking_count(&self, task_id: Uuid) -> usize {
        self.get_blocking_tasks(task_id).len()
    }

    /// Get configuration
    pub fn config(&self) -> &ValueScorerConfig {
        &self.config
    }

    /// Get total number of known spec requirements
    pub fn requirement_count(&self) -> usize {
        self.known_requirements.len()
    }

    /// Get all registered task IDs with spec links
    pub fn tasks_with_spec_links(&self) -> Vec<Uuid> {
        self.spec_requirements.keys().copied().collect()
    }

    /// Get all registered dependencies
    pub fn all_dependencies(&self) -> Vec<TaskDependency> {
        self.dependencies
            .iter()
            .flat_map(|(blocker, blocked_list)| {
                blocked_list
                    .iter()
                    .map(|blocked| TaskDependency {
                        blocker: *blocker,
                        blocked: *blocked,
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

impl Default for ValueScorer {
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

    // --- SpecAlignmentScore Tests ---

    #[test]
    fn test_spec_alignment_clamp() {
        let score = SpecAlignmentScore::new(0);
        assert_eq!(score.value(), 1);

        let score = SpecAlignmentScore::new(10);
        assert_eq!(score.value(), 5);

        let score = SpecAlignmentScore::new(3);
        assert_eq!(score.value(), 3);
    }

    #[test]
    fn test_spec_alignment_levels() {
        assert_eq!(SpecAlignmentScore::not_mentioned().value(), 1);
        assert_eq!(SpecAlignmentScore::tangential().value(), 2);
        assert_eq!(SpecAlignmentScore::supportive().value(), 3);
        assert_eq!(SpecAlignmentScore::implements().value(), 4);
        assert_eq!(SpecAlignmentScore::required().value(), 5);
    }

    #[test]
    fn test_spec_alignment_as_f32() {
        let score = SpecAlignmentScore::implements();
        assert!((score.as_f32() - 4.0).abs() < 0.001);
    }

    // --- BlockingImpactScore Tests ---

    #[test]
    fn test_blocking_impact_clamp() {
        let score = BlockingImpactScore::new(0);
        assert_eq!(score.value(), 1);

        let score = BlockingImpactScore::new(10);
        assert_eq!(score.value(), 5);

        let score = BlockingImpactScore::new(3);
        assert_eq!(score.value(), 3);
    }

    #[test]
    fn test_blocking_impact_levels() {
        assert_eq!(BlockingImpactScore::none().value(), 1);
        assert_eq!(BlockingImpactScore::few().value(), 2);
        assert_eq!(BlockingImpactScore::some().value(), 3);
        assert_eq!(BlockingImpactScore::multiple().value(), 4);
        assert_eq!(BlockingImpactScore::critical_path().value(), 5);
    }

    // --- TaskScore Tests ---

    #[test]
    fn test_task_score_computation() {
        let task_id = Uuid::new_v4();
        let score = TaskScore::new(
            task_id,
            SpecAlignmentScore::implements(), // 4
            BlockingImpactScore::multiple(),  // 4
            vec![],
            vec![],
        );

        // priority = 4 * 0.6 + 4 * 0.4 = 2.4 + 1.6 = 4.0
        assert!((score.priority_score - 4.0).abs() < 0.001);
    }

    #[test]
    fn test_task_score_priority_rounded() {
        let task_id = Uuid::new_v4();

        // 4.0 -> rounds to 4
        let score = TaskScore::new(
            task_id,
            SpecAlignmentScore::implements(),
            BlockingImpactScore::some(),
            vec![],
            vec![],
        );
        assert_eq!(score.priority_rounded(), 4);

        // 4.6 -> rounds to 5
        let score = TaskScore::new(
            task_id,
            SpecAlignmentScore::required(),
            BlockingImpactScore::critical_path(),
            vec![],
            vec![],
        );
        // priority = 5 * 0.6 + 5 * 0.4 = 5.0
        assert_eq!(score.priority_rounded(), 5);
    }

    // --- ValueScorer Tests ---

    #[test]
    fn test_score_task_no_links() {
        let scorer = ValueScorer::new();
        let task_id = Uuid::new_v4();

        let score = scorer.score_task(task_id);

        assert_eq!(score.spec_alignment.value(), 1);
        assert_eq!(score.blocking_impact.value(), 1);
    }

    #[test]
    fn test_score_task_with_spec_link() {
        let mut scorer = ValueScorer::new();
        let task_id = Uuid::new_v4();
        let req_id = Uuid::new_v4();

        scorer.register_spec_link(task_id, req_id);

        let score = scorer.score_task(task_id);

        assert_eq!(score.spec_alignment.value(), 4); // Implements spec
        assert_eq!(score.blocking_impact.value(), 1); // No blocking
    }

    #[test]
    fn test_score_task_with_blocking() {
        let mut scorer = ValueScorer::new();
        let blocker = Uuid::new_v4();
        let blocked = Uuid::new_v4();

        scorer.register_dependency(blocker, blocked);

        let score = scorer.score_task(blocker);

        assert_eq!(score.spec_alignment.value(), 1); // No spec link
        assert_eq!(score.blocking_impact.value(), 2); // 1 task blocked = few
        assert_eq!(score.blocks, vec![blocked]);
    }

    #[test]
    fn test_score_task_multiple_blockers() {
        let mut scorer = ValueScorer::new();
        let task = Uuid::new_v4();
        let blocked1 = Uuid::new_v4();
        let blocked2 = Uuid::new_v4();
        let blocked3 = Uuid::new_v4();

        scorer.register_dependency(task, blocked1);
        scorer.register_dependency(task, blocked2);
        scorer.register_dependency(task, blocked3);

        let score = scorer.score_task(task);

        assert_eq!(score.blocking_impact.value(), 3); // 3 tasks = some
        assert_eq!(score.blocks.len(), 3);
    }

    #[test]
    fn test_critical_path_blocking() {
        let mut scorer = ValueScorer::new();
        let task = Uuid::new_v4();
        // Create 7 blocked tasks (more than 6 = critical path)
        for _i in 0..7 {
            let _blocked = Uuid::new_v4();
            scorer.register_dependency(task, Uuid::new_v4());
        }

        let score = scorer.score_task(task);

        assert_eq!(score.blocking_impact.value(), 5); // Critical path
    }

    #[test]
    fn test_depends_on() {
        let mut scorer = ValueScorer::new();
        let blocker1 = Uuid::new_v4();
        let blocker2 = Uuid::new_v4();
        let blocked = Uuid::new_v4();

        scorer.register_dependency(blocker1, blocked);
        scorer.register_dependency(blocker2, blocked);

        let score = scorer.score_task(blocked);

        assert_eq!(score.depends_on.len(), 2);
        assert!(score.depends_on.contains(&blocker1));
        assert!(score.depends_on.contains(&blocker2));
    }

    #[test]
    fn test_supportive_task_blocks_spec_implementing() {
        let mut scorer = ValueScorer::new();
        let supportive = Uuid::new_v4();
        let implements = Uuid::new_v4();
        let req_id = Uuid::new_v4();

        // `implements` task implements a spec requirement
        scorer.register_spec_link(implements, req_id);
        // `supportive` task blocks `implements` (must complete before it runs)
        scorer.register_dependency(supportive, implements);

        let score = scorer.score_task(supportive);

        // Supportive task enables a spec-implementing task by completing first
        // So it's "supportive" of the spec requirement (enabling/supplementary)
        assert_eq!(score.spec_alignment.value(), 3); // Supportive
        assert_eq!(score.blocking_impact.value(), 2); // Blocks 1 task = few
    }

    #[test]
    fn test_score_tasks_batch() {
        let mut scorer = ValueScorer::new();
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();

        scorer.register_dependency(task1, task2);

        let scores = scorer.score_tasks(&[task1, task2]);

        assert_eq!(scores.len(), 2);
        // task1 blocks task2, so task1 has higher blocking impact
        assert!(scores[0].priority_score >= scores[1].priority_score);
    }

    #[test]
    fn test_blocked_count() {
        let mut scorer = ValueScorer::new();
        let task = Uuid::new_v4();

        assert_eq!(scorer.blocked_count(task), 0);

        scorer.register_dependency(task, Uuid::new_v4());
        scorer.register_dependency(task, Uuid::new_v4());

        assert_eq!(scorer.blocked_count(task), 2);
    }

    #[test]
    fn test_blocking_count() {
        let mut scorer = ValueScorer::new();
        let task = Uuid::new_v4();
        let blocker = Uuid::new_v4();

        assert_eq!(scorer.blocking_count(task), 0);

        scorer.register_dependency(blocker, task);

        assert_eq!(scorer.blocking_count(task), 1);
    }

    #[test]
    fn test_all_dependencies() {
        let mut scorer = ValueScorer::new();
        let dep1 = Uuid::new_v4();
        let dep2 = Uuid::new_v4();
        let dep3 = Uuid::new_v4();

        scorer.register_dependency(dep1, dep2);
        scorer.register_dependency(dep1, dep3);

        let all_deps = scorer.all_dependencies();

        assert_eq!(all_deps.len(), 2);
    }

    // --- Config Tests ---

    #[test]
    fn test_custom_weights() {
        let config = ValueScorerConfig {
            spec_weight: 0.7,
            blocking_weight: 0.3,
        };
        let scorer = ValueScorer::with_config(config);

        assert!((scorer.config().spec_weight - 0.7).abs() < 0.001);
        assert!((scorer.config().blocking_weight - 0.3).abs() < 0.001);
    }

    #[test]
    fn test_weights_must_sum_to_one() {
        let config = ValueScorerConfig {
            spec_weight: 0.5,
            blocking_weight: 0.5,
        };
        let _scorer = ValueScorer::with_config(config); // Should not panic

        let bad_config = ValueScorerConfig {
            spec_weight: 0.3,
            blocking_weight: 0.3,
        };
        // This should panic due to assertion
        let result = std::panic::catch_unwind(|| ValueScorer::with_config(bad_config));
        assert!(result.is_err());
    }

    // --- Edge Cases ---

    #[test]
    fn test_empty_task_id() {
        let scorer = ValueScorer::new();
        let nil_uuid = Uuid::nil();

        let score = scorer.score_task(nil_uuid);

        assert_eq!(score.spec_alignment.value(), 1);
        assert_eq!(score.blocking_impact.value(), 1);
    }

    #[test]
    fn test_self_dependency() {
        let mut scorer = ValueScorer::new();
        let task = Uuid::new_v4();

        // Register self as blocking itself (edge case)
        scorer.register_dependency(task, task);

        let score = scorer.score_task(task);

        // Self-dependency counts as 1 blocked task
        assert_eq!(score.blocking_impact.value(), 2); // 1 task blocked = few
        assert_eq!(score.blocks, vec![task]);
        assert!(score.depends_on.contains(&task));
        // No spec link, so spec alignment is 1 (not mentioned)
        assert_eq!(score.spec_alignment.value(), 1);
    }
}
