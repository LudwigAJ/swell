//! Value Scorer - scores tasks 1-5 on spec alignment, blocking impact, and complexity for prioritization.
//!
//! This module provides functionality to:
//! - Score tasks 1-5 on spec alignment (how well task matches the spec)
//! - Score tasks 1-5 on blocking impact (how much task blocks other work)
//! - Score tasks 1-5 on complexity (estimated implementation complexity)
//! - Combine scores into a single priority score for task scheduling
//! - Filter tasks below configurable discard threshold
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
//! ## Complexity Score
//! - 5: Very complex (large refactor, many files, intricate logic)
//! - 4: Complex (multiple modules, significant changes)
//! - 3: Moderate complexity (standard implementation)
//! - 2: Low complexity (small change, isolated)
//! - 1: Trivial (single file, obvious fix)
//!
//! # Combined Score
//!
//! The combined priority score is a weighted average:
//! `priority = spec_alignment * spec_weight + blocking_impact * blocking_weight + (6 - complexity) * complexity_weight`
//!
//! Note: Complexity is inverted (6 - complexity) so that lower complexity = higher priority.
//! Default weights: spec=0.4, blocking=0.3, complexity=0.3

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use swell_core::ids::TaskId;
use tracing::{debug, info};
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

/// Complexity score level (1-5)
///
/// Higher scores mean more complex tasks. Complexity is inverted when computing
/// priority (i.e., `6 - complexity` so that lower complexity = higher priority).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComplexityScore(u8);

impl ComplexityScore {
    /// Create a new complexity score (clamps to 1-5)
    pub fn new(score: u8) -> Self {
        Self(score.clamp(1, 5))
    }

    /// Score 1: Trivial (single file, obvious fix)
    pub fn trivial() -> Self {
        Self(1)
    }

    /// Score 2: Low complexity (small change, isolated)
    pub fn low() -> Self {
        Self(2)
    }

    /// Score 3: Moderate complexity (standard implementation)
    pub fn moderate() -> Self {
        Self(3)
    }

    /// Score 4: Complex (multiple modules, significant changes)
    pub fn complex() -> Self {
        Self(4)
    }

    /// Score 5: Very complex (large refactor, many files, intricate logic)
    pub fn very_complex() -> Self {
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

    /// Get the inverted score (6 - value) for priority computation
    /// This inverts complexity so that lower complexity = higher priority
    pub fn inverted(&self) -> f32 {
        (6 - self.0) as f32
    }
}

impl Default for ComplexityScore {
    fn default() -> Self {
        Self::moderate()
    }
}

impl std::fmt::Display for ComplexityScore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A task dependency relationship for blocking analysis
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TaskDependency {
    /// The blocking task ID
    pub blocker: TaskId,
    /// The blocked task ID
    pub blocked: TaskId,
}

/// Result of scoring a task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskScore {
    /// Task ID
    pub task_id: TaskId,
    /// Spec alignment score (1-5)
    pub spec_alignment: SpecAlignmentScore,
    /// Blocking impact score (1-5)
    pub blocking_impact: BlockingImpactScore,
    /// Complexity score (1-5, higher = more complex)
    pub complexity: ComplexityScore,
    /// Combined priority score (1-5, weighted average)
    pub priority_score: f32,
    /// Which other tasks this task blocks
    pub blocks: Vec<TaskId>,
    /// Which tasks this task depends on
    pub depends_on: Vec<TaskId>,
}

impl TaskScore {
    /// Create a new task score with all three dimensions
    pub fn new(
        task_id: TaskId,
        spec_alignment: SpecAlignmentScore,
        blocking_impact: BlockingImpactScore,
        complexity: ComplexityScore,
        blocks: Vec<TaskId>,
        depends_on: Vec<TaskId>,
    ) -> Self {
        let priority_score = Self::compute_priority(spec_alignment, blocking_impact, complexity);
        Self {
            task_id,
            spec_alignment,
            blocking_impact,
            complexity,
            priority_score,
            blocks,
            depends_on,
        }
    }

    /// Compute the combined priority score
    ///
    /// Formula: priority = spec * spec_weight + blocking * blocking_weight + (6 - complexity) * complexity_weight
    /// Note: Complexity is inverted (6 - complexity) so lower complexity = higher priority
    fn compute_priority(
        spec_alignment: SpecAlignmentScore,
        blocking_impact: BlockingImpactScore,
        complexity: ComplexityScore,
    ) -> f32 {
        // Default weights: spec=0.4, blocking=0.3, complexity=0.3
        let spec_weight = 0.4;
        let blocking_weight = 0.3;
        let complexity_weight = 0.3;

        spec_alignment.as_f32() * spec_weight
            + blocking_impact.as_f32() * blocking_weight
            + complexity.inverted() * complexity_weight
    }

    /// Get the priority as an integer (rounded)
    pub fn priority_rounded(&self) -> u8 {
        (self.priority_score + 0.5) as u8
    }

    /// Check if this task should be discarded based on threshold
    pub fn is_discarded(&self, threshold: f32) -> bool {
        self.priority_score < threshold
    }
}

/// Configuration for value scoring
#[derive(Debug, Clone)]
pub struct ValueScorerConfig {
    /// Weight for spec alignment in combined score (0.0-1.0)
    pub spec_weight: f32,
    /// Weight for blocking impact in combined score (0.0-1.0)
    pub blocking_weight: f32,
    /// Weight for complexity in combined score (0.0-1.0)
    /// Note: Complexity is inverted so lower complexity = higher priority
    pub complexity_weight: f32,
    /// Minimum priority score to keep a task (tasks below this are discarded)
    /// Default is 2.0
    pub discard_threshold: f32,
}

impl Default for ValueScorerConfig {
    fn default() -> Self {
        Self {
            spec_weight: 0.4,
            blocking_weight: 0.3,
            complexity_weight: 0.3,
            discard_threshold: 2.0,
        }
    }
}

/// Value Scorer for scoring tasks on spec alignment, blocking impact, and complexity
pub struct ValueScorer {
    config: ValueScorerConfig,
    /// Map of task ID to spec requirement IDs it fulfills
    spec_requirements: HashMap<TaskId, Vec<Uuid>>,
    /// Map of spec requirement ID to task IDs that implement it
    requirement_implementers: HashMap<Uuid, Vec<TaskId>>,
    /// Known spec requirements
    known_requirements: HashSet<Uuid>,
    /// Dependency graph (blocker -> blocked)
    dependencies: HashMap<TaskId, Vec<TaskId>>,
    /// Explicit complexity scores for tasks
    complexity_scores: HashMap<TaskId, ComplexityScore>,
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
        let complexity_weight = config.complexity_weight;
        // Ensure weights sum to 1.0
        let total = spec_weight + blocking_weight + complexity_weight;
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
            complexity_scores: HashMap::new(),
        }
    }

    /// Register a task as implementing a spec requirement
    pub fn register_spec_link(&mut self, task_id: TaskId, requirement_id: Uuid) {
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
    pub fn register_dependency(&mut self, blocker: TaskId, blocked: TaskId) {
        debug!(
            blocker = %blocker,
            blocked = %blocked,
            "Registering dependency"
        );
        self.dependencies.entry(blocker).or_default().push(blocked);
    }

    /// Set explicit complexity score for a task
    ///
    /// Use this to provide estimated complexity from task description analysis
    /// or dependency analysis. If not set, defaults to moderate (3).
    pub fn set_complexity(&mut self, task_id: TaskId, complexity: ComplexityScore) {
        debug!(
            task_id = %task_id,
            complexity = %complexity,
            "Setting complexity score"
        );
        self.complexity_scores.insert(task_id, complexity);
    }

    /// Register a dependency and set complexity for the blocking task
    pub fn register_dependency_with_complexity(
        &mut self,
        blocker: TaskId,
        blocked: TaskId,
        complexity: ComplexityScore,
    ) {
        self.register_dependency(blocker, blocked);
        self.set_complexity(blocker, complexity);
    }

    /// Score a task based on spec alignment, blocking impact, and complexity
    pub fn score_task(&self, task_id: TaskId) -> TaskScore {
        let spec_alignment = self.compute_spec_alignment(task_id);
        let blocking_impact = self.compute_blocking_impact(task_id);
        let complexity = self.get_complexity(task_id);
        let blocks = self.get_blocked_tasks(task_id);
        let depends_on = self.get_blocking_tasks(task_id);

        TaskScore::new(
            task_id,
            spec_alignment,
            blocking_impact,
            complexity,
            blocks,
            depends_on,
        )
    }

    /// Score multiple tasks
    pub fn score_tasks(&self, task_ids: &[TaskId]) -> Vec<TaskScore> {
        task_ids.iter().map(|id| self.score_task(*id)).collect()
    }

    /// Compute spec alignment score (1-5)
    fn compute_spec_alignment(&self, task_id: TaskId) -> SpecAlignmentScore {
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
    fn count_enables_spec_fulfilling(&self, task_id: TaskId) -> usize {
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
    fn blocks_tasks_with_spec_links(&self, task_id: TaskId) -> bool {
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
    fn compute_blocking_impact(&self, task_id: TaskId) -> BlockingImpactScore {
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
    pub fn get_blocked_tasks(&self, task_id: TaskId) -> Vec<TaskId> {
        self.dependencies.get(&task_id).cloned().unwrap_or_default()
    }

    /// Get tasks that this task depends on (tasks that block it)
    pub fn get_blocking_tasks(&self, task_id: TaskId) -> Vec<TaskId> {
        let mut blocking = Vec::new();
        for (blocker, blocked_list) in &self.dependencies {
            if blocked_list.contains(&task_id) {
                blocking.push(*blocker);
            }
        }
        blocking
    }

    /// Get the number of tasks blocked by a task
    pub fn blocked_count(&self, task_id: TaskId) -> usize {
        self.dependencies
            .get(&task_id)
            .map(|v| v.len())
            .unwrap_or(0)
    }

    /// Get the number of tasks a task depends on
    pub fn blocking_count(&self, task_id: TaskId) -> usize {
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
    pub fn tasks_with_spec_links(&self) -> Vec<TaskId> {
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

    /// Get complexity score for a task (explicit or default)
    fn get_complexity(&self, task_id: TaskId) -> ComplexityScore {
        self.complexity_scores
            .get(&task_id)
            .copied()
            .unwrap_or_default()
    }

    /// Score tasks and filter by discard threshold
    ///
    /// Returns (kept_scores, discarded_scores) tuple where:
    /// - kept_scores: tasks with priority >= discard_threshold
    /// - discarded_scores: tasks with priority < discard_threshold
    ///
    /// Discarded tasks are logged with reason and their scores.
    pub fn score_and_filter(&self, task_ids: &[TaskId]) -> (Vec<TaskScore>, Vec<TaskScore>) {
        let threshold = self.config.discard_threshold;
        let mut kept = Vec::new();
        let mut discarded = Vec::new();

        for task_id in task_ids {
            let score = self.score_task(*task_id);
            if score.is_discarded(threshold) {
                debug!(
                    task_id = %task_id,
                    priority_score = %score.priority_score,
                    threshold = %threshold,
                    spec_alignment = %score.spec_alignment,
                    blocking_impact = %score.blocking_impact,
                    complexity = %score.complexity,
                    "Task discarded: below priority threshold"
                );
                discarded.push(score);
            } else {
                kept.push(score);
            }
        }

        if !discarded.is_empty() {
            info!(
                discarded_count = discarded.len(),
                kept_count = kept.len(),
                threshold = %threshold,
                "Filtered tasks by discard threshold"
            );
        }

        (kept, discarded)
    }

    /// Get the configured discard threshold
    pub fn discard_threshold(&self) -> f32 {
        self.config.discard_threshold
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
        let task_id = TaskId::new();
        let score = TaskScore::new(
            task_id,
            SpecAlignmentScore::implements(), // 4
            BlockingImpactScore::multiple(),  // 4
            ComplexityScore::moderate(),      // 3 (inverted to 3)
            vec![],
            vec![],
        );

        // priority = 4 * 0.4 + 4 * 0.3 + (6-3) * 0.3 = 1.6 + 1.2 + 0.9 = 3.7
        assert!((score.priority_score - 3.7).abs() < 0.001);
    }

    #[test]
    fn test_task_score_priority_rounded() {
        let task_id = TaskId::new();

        // 3.0 -> rounds to 3
        let score = TaskScore::new(
            task_id,
            SpecAlignmentScore::implements(),
            BlockingImpactScore::some(),
            ComplexityScore::moderate(),
            vec![],
            vec![],
        );
        assert_eq!(score.priority_rounded(), 3);

        // 5 * 0.4 + 5 * 0.3 + (6-1) * 0.3 = 2.0 + 1.5 + 1.5 = 5.0
        let score = TaskScore::new(
            task_id,
            SpecAlignmentScore::required(),
            BlockingImpactScore::critical_path(),
            ComplexityScore::trivial(), // complexity 1, inverted = 5
            vec![],
            vec![],
        );
        assert_eq!(score.priority_rounded(), 5);
    }

    // --- ValueScorer Tests ---

    #[test]
    fn test_score_task_no_links() {
        let scorer = ValueScorer::new();
        let task_id = TaskId::new();

        let score = scorer.score_task(task_id);

        assert_eq!(score.spec_alignment.value(), 1);
        assert_eq!(score.blocking_impact.value(), 1);
    }

    #[test]
    fn test_score_task_with_spec_link() {
        let mut scorer = ValueScorer::new();
        let task_id = TaskId::new();
        let req_id = Uuid::new_v4();

        scorer.register_spec_link(task_id, req_id);

        let score = scorer.score_task(task_id);

        assert_eq!(score.spec_alignment.value(), 4); // Implements spec
        assert_eq!(score.blocking_impact.value(), 1); // No blocking
    }

    #[test]
    fn test_score_task_with_blocking() {
        let mut scorer = ValueScorer::new();
        let blocker = TaskId::new();
        let blocked = TaskId::new();

        scorer.register_dependency(blocker, blocked);

        let score = scorer.score_task(blocker);

        assert_eq!(score.spec_alignment.value(), 1); // No spec link
        assert_eq!(score.blocking_impact.value(), 2); // 1 task blocked = few
        assert_eq!(score.blocks, vec![blocked]);
    }

    #[test]
    fn test_score_task_multiple_blockers() {
        let mut scorer = ValueScorer::new();
        let task = TaskId::new();
        let blocked1 = TaskId::new();
        let blocked2 = TaskId::new();
        let blocked3 = TaskId::new();

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
        let task = TaskId::new();
        // Create 7 blocked tasks (more than 6 = critical path)
        for _i in 0..7 {
            let _blocked = TaskId::new();
            scorer.register_dependency(task, TaskId::new());
        }

        let score = scorer.score_task(task);

        assert_eq!(score.blocking_impact.value(), 5); // Critical path
    }

    #[test]
    fn test_depends_on() {
        let mut scorer = ValueScorer::new();
        let blocker1 = TaskId::new();
        let blocker2 = TaskId::new();
        let blocked = TaskId::new();

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
        let supportive = TaskId::new();
        let implements = TaskId::new();
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
        let task1 = TaskId::new();
        let task2 = TaskId::new();

        scorer.register_dependency(task1, task2);

        let scores = scorer.score_tasks(&[task1, task2]);

        assert_eq!(scores.len(), 2);
        // task1 blocks task2, so task1 has higher blocking impact
        assert!(scores[0].priority_score >= scores[1].priority_score);
    }

    #[test]
    fn test_blocked_count() {
        let mut scorer = ValueScorer::new();
        let task = TaskId::new();

        assert_eq!(scorer.blocked_count(task), 0);

        scorer.register_dependency(task, TaskId::new());
        scorer.register_dependency(task, TaskId::new());

        assert_eq!(scorer.blocked_count(task), 2);
    }

    #[test]
    fn test_blocking_count() {
        let mut scorer = ValueScorer::new();
        let task = TaskId::new();
        let blocker = TaskId::new();

        assert_eq!(scorer.blocking_count(task), 0);

        scorer.register_dependency(blocker, task);

        assert_eq!(scorer.blocking_count(task), 1);
    }

    #[test]
    fn test_all_dependencies() {
        let mut scorer = ValueScorer::new();
        let dep1 = TaskId::new();
        let dep2 = TaskId::new();
        let dep3 = TaskId::new();

        scorer.register_dependency(dep1, dep2);
        scorer.register_dependency(dep1, dep3);

        let all_deps = scorer.all_dependencies();

        assert_eq!(all_deps.len(), 2);
    }

    // --- Config Tests ---

    #[test]
    fn test_custom_weights() {
        let config = ValueScorerConfig {
            spec_weight: 0.5,
            blocking_weight: 0.3,
            complexity_weight: 0.2,
            discard_threshold: 2.5,
        };
        let scorer = ValueScorer::with_config(config);

        assert!((scorer.config().spec_weight - 0.5).abs() < 0.001);
        assert!((scorer.config().blocking_weight - 0.3).abs() < 0.001);
        assert!((scorer.config().complexity_weight - 0.2).abs() < 0.001);
        assert!((scorer.config().discard_threshold - 2.5).abs() < 0.001);
    }

    #[test]
    fn test_weights_must_sum_to_one() {
        let config = ValueScorerConfig {
            spec_weight: 0.4,
            blocking_weight: 0.3,
            complexity_weight: 0.3,
            discard_threshold: 2.0,
        };
        let _scorer = ValueScorer::with_config(config); // Should not panic

        let bad_config = ValueScorerConfig {
            spec_weight: 0.3,
            blocking_weight: 0.3,
            complexity_weight: 0.3,
            discard_threshold: 2.0,
        };
        // This should panic due to assertion (0.3+0.3+0.3 = 0.9, not 1.0)
        let result = std::panic::catch_unwind(|| ValueScorer::with_config(bad_config));
        assert!(result.is_err());
    }

    // --- Edge Cases ---

    #[test]
    fn test_empty_task_id() {
        let scorer = ValueScorer::new();
        let nil_uuid = TaskId::from_uuid(Uuid::nil());

        let score = scorer.score_task(nil_uuid);

        assert_eq!(score.spec_alignment.value(), 1);
        assert_eq!(score.blocking_impact.value(), 1);
    }

    // --- ComplexityScore Tests ---

    #[test]
    fn test_complexity_score_clamp() {
        let score = ComplexityScore::new(0);
        assert_eq!(score.value(), 1);

        let score = ComplexityScore::new(10);
        assert_eq!(score.value(), 5);

        let score = ComplexityScore::new(3);
        assert_eq!(score.value(), 3);
    }

    #[test]
    fn test_complexity_score_levels() {
        assert_eq!(ComplexityScore::trivial().value(), 1);
        assert_eq!(ComplexityScore::low().value(), 2);
        assert_eq!(ComplexityScore::moderate().value(), 3);
        assert_eq!(ComplexityScore::complex().value(), 4);
        assert_eq!(ComplexityScore::very_complex().value(), 5);
    }

    #[test]
    fn test_complexity_inverted() {
        // Lower complexity = higher inverted score
        assert!((ComplexityScore::trivial().inverted() - 5.0).abs() < 0.001); // 6-1=5
        assert!((ComplexityScore::low().inverted() - 4.0).abs() < 0.001); // 6-2=4
        assert!((ComplexityScore::moderate().inverted() - 3.0).abs() < 0.001); // 6-3=3
        assert!((ComplexityScore::complex().inverted() - 2.0).abs() < 0.001); // 6-4=2
        assert!((ComplexityScore::very_complex().inverted() - 1.0).abs() < 0.001);
        // 6-5=1
    }

    // --- Complexity in Priority Computation Tests ---

    #[test]
    fn test_complexity_affects_priority() {
        let task_id = TaskId::new();
        let spec = SpecAlignmentScore::implements(); // 4
        let blocking = BlockingImpactScore::some(); // 3

        // Trivial complexity (1) -> inverted is 5
        let score_trivial = TaskScore::new(
            task_id,
            spec,
            blocking,
            ComplexityScore::trivial(),
            vec![],
            vec![],
        );
        // 4 * 0.4 + 3 * 0.3 + 5 * 0.3 = 1.6 + 0.9 + 1.5 = 4.0

        // Very complex (5) -> inverted is 1
        let score_complex = TaskScore::new(
            task_id,
            spec,
            blocking,
            ComplexityScore::very_complex(),
            vec![],
            vec![],
        );
        // 4 * 0.4 + 3 * 0.3 + 1 * 0.3 = 1.6 + 0.9 + 0.3 = 2.8

        // Trivial should have higher priority than complex
        assert!(score_trivial.priority_score > score_complex.priority_score);
        assert!((score_trivial.priority_score - 4.0).abs() < 0.001);
        assert!((score_complex.priority_score - 2.8).abs() < 0.001);
    }

    // --- is_discarded Tests ---

    #[test]
    fn test_is_discarded() {
        let task_id = TaskId::new();
        let score = TaskScore::new(
            task_id,
            SpecAlignmentScore::not_mentioned(), // 1
            BlockingImpactScore::none(),         // 1
            ComplexityScore::very_complex(),     // 5, inverted = 1
            vec![],
            vec![],
        );
        // 1 * 0.4 + 1 * 0.3 + 1 * 0.3 = 0.4 + 0.3 + 0.3 = 1.0

        assert!(score.is_discarded(2.0)); // 1.0 < 2.0
        assert!(!score.is_discarded(1.0)); // 1.0 >= 1.0
    }

    #[test]
    fn test_is_not_discarded_high_score() {
        let task_id = TaskId::new();
        let score = TaskScore::new(
            task_id,
            SpecAlignmentScore::required(),       // 5
            BlockingImpactScore::critical_path(), // 5
            ComplexityScore::trivial(),           // 1, inverted = 5
            vec![],
            vec![],
        );
        // 5 * 0.4 + 5 * 0.3 + 5 * 0.3 = 2.0 + 1.5 + 1.5 = 5.0

        assert!(!score.is_discarded(2.0)); // 5.0 >= 2.0
    }

    // --- score_and_filter Tests ---

    #[test]
    fn test_score_and_filter_all_above_threshold() {
        let mut scorer = ValueScorer::new();
        let task1 = TaskId::new();
        let task2 = TaskId::new();

        // Give both tasks high spec alignment
        scorer.register_spec_link(task1, Uuid::new_v4());
        scorer.register_spec_link(task2, Uuid::new_v4());

        let (kept, discarded) = scorer.score_and_filter(&[task1, task2]);

        assert_eq!(kept.len(), 2);
        assert!(discarded.is_empty());
    }

    #[test]
    fn test_score_and_filter_some_discarded() {
        let mut scorer = ValueScorer::new();
        let task1 = TaskId::new();
        let task2 = TaskId::new();

        // task1 has spec link (high priority)
        scorer.register_spec_link(task1, Uuid::new_v4());
        // task2 has nothing (low priority)

        let (kept, _discarded) = scorer.score_and_filter(&[task1, task2]);

        // task1 should be kept, task2 should be discarded
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].task_id, task1);
    }

    #[test]
    fn test_score_and_filter_custom_threshold() {
        let config = ValueScorerConfig {
            spec_weight: 0.4,
            blocking_weight: 0.3,
            complexity_weight: 0.3,
            discard_threshold: 4.0, // High threshold
        };
        let mut scorer = ValueScorer::with_config(config);
        let task1 = TaskId::new();
        let task2 = TaskId::new();

        // task1 has spec link (should give priority ~4.0)
        scorer.register_spec_link(task1, Uuid::new_v4());
        // task2 has nothing

        let (kept, _discarded) = scorer.score_and_filter(&[task1, task2]);

        // With threshold 4.0, even spec-linked task might be discarded
        // spec_link gives spec=4, blocking=1, complexity=3(default)
        // 4 * 0.4 + 1 * 0.3 + 3 * 0.3 = 1.6 + 0.3 + 0.9 = 2.8
        // 2.8 < 4.0, so both should be discarded
        assert_eq!(kept.len(), 0);
        assert_eq!(_discarded.len(), 2);
    }

    #[test]
    fn test_score_and_filter_empty_input() {
        let scorer = ValueScorer::new();
        let (kept, discarded) = scorer.score_and_filter(&[]);

        assert!(kept.is_empty());
        assert!(discarded.is_empty());
    }

    #[test]
    fn test_score_and_filter_none_discarded() {
        let config = ValueScorerConfig {
            spec_weight: 0.4,
            blocking_weight: 0.3,
            complexity_weight: 0.3,
            discard_threshold: 1.0, // Very low threshold
        };
        let scorer = ValueScorer::with_config(config);
        let task_id = TaskId::new();

        let (kept, discarded) = scorer.score_and_filter(&[task_id]);

        // With threshold 1.0, even task with priority 1.0 should be kept
        assert_eq!(kept.len(), 1);
        assert!(discarded.is_empty());
    }

    // --- Complexity Registration Tests ---

    #[test]
    fn test_set_complexity() {
        let mut scorer = ValueScorer::new();
        let task_id = TaskId::new();

        scorer.set_complexity(task_id, ComplexityScore::very_complex());

        let score = scorer.score_task(task_id);
        assert_eq!(score.complexity.value(), 5);
    }

    #[test]
    fn test_complexity_defaults_to_moderate() {
        let scorer = ValueScorer::new();
        let task_id = TaskId::new();

        let score = scorer.score_task(task_id);
        assert_eq!(score.complexity.value(), 3); // Default is moderate
    }

    #[test]
    fn test_register_dependency_with_complexity() {
        let mut scorer = ValueScorer::new();
        let blocker = TaskId::new();
        let blocked = TaskId::new();

        scorer.register_dependency_with_complexity(blocker, blocked, ComplexityScore::complex());

        let score = scorer.score_task(blocker);
        assert_eq!(score.complexity.value(), 4);
    }

    #[test]
    fn test_complexity_affects_filtering() {
        let mut scorer = ValueScorer::new();
        let task1 = TaskId::new();
        let task2 = TaskId::new();

        // Both have spec alignment
        scorer.register_spec_link(task1, Uuid::new_v4());
        scorer.register_spec_link(task2, Uuid::new_v4());

        // But different complexity
        scorer.set_complexity(task1, ComplexityScore::trivial()); // High priority
        scorer.set_complexity(task2, ComplexityScore::very_complex()); // Low priority

        let (kept, discarded) = scorer.score_and_filter(&[task1, task2]);

        // task1 should be kept (high priority due to low complexity)
        // task2 might be discarded depending on threshold
        // With spec=4, blocking=1:
        // task1: 4*0.4 + 1*0.3 + 5*0.3 = 1.6 + 0.3 + 1.5 = 3.4
        // task2: 4*0.4 + 1*0.3 + 1*0.3 = 1.6 + 0.3 + 0.3 = 2.2
        // Both >= 2.0, so both kept
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn test_score_and_filter_some_discarded_v2() {
        let mut scorer = ValueScorer::new();
        let task1 = TaskId::new();
        let task2 = TaskId::new();

        // task1 has spec link (high priority)
        scorer.register_spec_link(task1, Uuid::new_v4());
        // task2 has nothing

        let (kept, _discarded) = scorer.score_and_filter(&[task1, task2]);

        // task1 should be kept, task2 should be discarded
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].task_id, task1);
    }

    // --- discard_threshold accessor test ---

    #[test]
    fn test_discard_threshold_accessor() {
        let config = ValueScorerConfig {
            spec_weight: 0.4,
            blocking_weight: 0.3,
            complexity_weight: 0.3,
            discard_threshold: 2.5,
        };
        let scorer = ValueScorer::with_config(config);

        assert!((scorer.discard_threshold() - 2.5).abs() < 0.001);
    }

    // --- Default config test ---

    #[test]
    fn test_default_config_has_three_weights() {
        let config = ValueScorerConfig::default();

        // Check all three weights are set
        assert!((config.spec_weight - 0.4).abs() < 0.001);
        assert!((config.blocking_weight - 0.3).abs() < 0.001);
        assert!((config.complexity_weight - 0.3).abs() < 0.001);
        assert!((config.discard_threshold - 2.0).abs() < 0.001);

        // Weights should sum to 1.0
        let total = config.spec_weight + config.blocking_weight + config.complexity_weight;
        assert!((total - 1.0).abs() < 0.001);
    }

    // --- Edge case: score_and_filter with explicit complexity ---

    #[test]
    fn test_score_and_filter_with_explicit_complexity() {
        let mut scorer = ValueScorer::new();
        let task1 = TaskId::new();
        let task2 = TaskId::new();
        let task3 = TaskId::new();

        // task1: spec aligned, low complexity -> should be kept
        scorer.register_spec_link(task1, Uuid::new_v4());
        scorer.set_complexity(task1, ComplexityScore::trivial());

        // task2: spec aligned, high complexity -> might be discarded
        scorer.register_spec_link(task2, Uuid::new_v4());
        scorer.set_complexity(task2, ComplexityScore::very_complex());

        // task3: no spec alignment, high complexity -> should be discarded
        scorer.set_complexity(task3, ComplexityScore::very_complex());

        let (kept, discarded) = scorer.score_and_filter(&[task1, task2, task3]);

        // task1 should definitely be kept (spec=4, complexity=1 -> inverted=5)
        // priority = 4*0.4 + 1*0.3 + 5*0.3 = 1.6 + 0.3 + 1.5 = 3.4
        assert!(kept.iter().any(|s| s.task_id == task1));

        // task3 should definitely be discarded (spec=1, complexity=5 -> inverted=1)
        // priority = 1*0.4 + 1*0.3 + 1*0.3 = 0.4 + 0.3 + 0.3 = 1.0
        assert!(discarded.iter().any(|s| s.task_id == task3));
    }

    #[test]
    fn test_self_dependency() {
        let mut scorer = ValueScorer::new();
        let task = TaskId::new();

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
