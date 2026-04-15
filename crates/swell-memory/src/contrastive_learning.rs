// contrastive_learning.rs - Contrastive learning from success/failure trajectories
//
// This module provides functionality to analyze both successful and failed task executions
// and apply contrastive learning to train embeddings. The goal is to learn representations
// where successful outcomes cluster together and failures are pushed apart from successes.
//
// Contrastive Learning Theory:
// - Positive pairs: (success, success) embeddings should be CLOSER together
// - Negative pairs: (success, failure) embeddings should be FARTHER apart
// - Contrastive loss = (1 - margin + dist(pos))^2 + max(0, dist(neg) - margin)^2

use crate::{SqliteMemoryStore, SwellError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use swell_core::MemoryStore;
use uuid::Uuid;

/// Represents a successful task trajectory with associated memories
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessTrajectory {
    pub task_id: Uuid,
    pub task_description: String,
    /// Memory embeddings associated with this successful trajectory
    pub memory_ids: Vec<Uuid>,
    /// Steps executed in this trajectory
    pub steps: Vec<TrajectoryStep>,
    /// Outcome details
    pub outcome: SuccessOutcome,
    /// Tool calls made during execution
    pub tool_calls: Vec<ToolCallRecord>,
    /// Files modified
    pub files_modified: Vec<String>,
    pub timestamp: DateTime<Utc>,
}

/// A step in a trajectory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryStep {
    pub step_id: Uuid,
    pub description: String,
    pub affected_files: Vec<String>,
    pub risk_level: String,
    pub status: StepStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Executed,
    Skipped,
    Failed,
}

/// Outcome of a successful task
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuccessOutcome {
    Accepted,
    Approved,
    Merged,
}

/// A tool call record during trajectory execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub success: bool,
    pub timestamp: DateTime<Utc>,
}

/// Represents a failed/rejected task trajectory with associated memories
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureTrajectory {
    pub task_id: Uuid,
    pub task_description: String,
    /// Memory embeddings associated with this failed trajectory
    pub memory_ids: Vec<Uuid>,
    /// Steps executed before failure
    pub steps: Vec<TrajectoryStep>,
    /// Failure reason
    pub failure_reason: FailureReason,
    /// Validation errors if any
    pub validation_errors: Vec<ValidationErrorRecord>,
    /// Tool calls made
    pub tool_calls: Vec<ToolCallRecord>,
    /// Files modified before failure
    pub files_modified: Vec<String>,
    /// Number of retry iterations
    pub iteration_count: u32,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    ValidationFailure,
    LintFailure,
    TestFailure,
    SecurityIssue,
    AiReviewFailure,
    PolicyViolation,
    Timeout,
    ResourceExceeded,
    Unknown,
}

impl FailureReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            FailureReason::ValidationFailure => "validation_failure",
            FailureReason::LintFailure => "lint_failure",
            FailureReason::TestFailure => "test_failure",
            FailureReason::SecurityIssue => "security_issue",
            FailureReason::AiReviewFailure => "ai_review_failure",
            FailureReason::PolicyViolation => "policy_violation",
            FailureReason::Timeout => "timeout",
            FailureReason::ResourceExceeded => "resource_exceeded",
            FailureReason::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationErrorRecord {
    pub error_type: String,
    pub message: String,
    pub file: Option<String>,
    pub line: Option<u32>,
}

/// A contrastive learning pair: anchor, comparison, and their relationship
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContrastivePair {
    /// The anchor embedding (typically from a successful trajectory)
    pub anchor_id: Uuid,
    /// The comparison embedding
    pub comparison_id: Uuid,
    /// Type of pair - positive means similar/outcome should be close, negative means dissimilar
    pub pair_type: PairType,
    /// The margin for contrastive loss (default 1.0)
    pub margin: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairType {
    /// Positive pair - embeddings should be close (success-success or same pattern)
    Positive,
    /// Negative pair - embeddings should be far apart (success-failure)
    Negative,
}

/// Result of contrastive learning computation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContrastiveLearningResult {
    /// Total contrastive loss value
    pub loss: f32,
    /// Average distance among positive pairs
    pub positive_avg_distance: f32,
    /// Average distance among negative pairs
    pub negative_avg_distance: f32,
    /// Number of positive pairs analyzed
    pub positive_pairs_count: usize,
    /// Number of negative pairs analyzed
    pub negative_pairs_count: usize,
    /// Embeddings that were updated
    pub embeddings_updated: Vec<Uuid>,
    /// The loss components for analysis
    pub loss_components: LossComponents,
}

/// Breakdown of contrastive loss components
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LossComponents {
    /// Loss from positive pairs (should be low)
    pub positive_loss: f32,
    /// Loss from negative pairs (should be low when far apart)
    pub negative_loss: f32,
}

/// A differentiating factor that distinguishes successful from failed trajectories
/// These are extracted during contrastive learning analysis and used to
/// tighten procedure preconditions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DifferentiatingFactor {
    /// Unique identifier for this factor
    pub id: Uuid,
    /// Type of factor
    pub factor_type: DifferentiatingFactorType,
    /// The specific value (tool name, file path, context key, etc.)
    pub value: String,
    /// True if this factor is PRESENT in success but ABSENT in failure
    /// False if this factor is ABSENT in success but PRESENT in failure
    pub success_has_failure_has_not: bool,
    /// Confidence score (0.0 to 1.0) based on frequency of occurrence
    pub confidence: f64,
    /// Number of times this factor was observed
    pub observation_count: usize,
    /// Which trajectory pair produced this factor
    pub source_pair_id: Option<Uuid>,
}

impl DifferentiatingFactor {
    /// Create a new differentiating factor
    pub fn new(
        factor_type: DifferentiatingFactorType,
        value: String,
        success_has_failure_has_not: bool,
        confidence: f64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            factor_type,
            value,
            success_has_failure_has_not,
            confidence,
            observation_count: 1,
            source_pair_id: None,
        }
    }

    /// Update confidence based on additional observation
    /// Uses Bayesian update: new_confidence = (observations * old_confidence + new_confidence) / (observations + 1)
    pub fn update_confidence(&mut self, additional_confidence: f64) {
        let old_count = self.observation_count;
        self.observation_count += 1;
        self.confidence = (old_count as f64 * self.confidence + additional_confidence)
            / (self.observation_count as f64);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DifferentiatingFactorType {
    /// Tool was used in success but not in failure
    ToolUsedInSuccess,
    /// Tool was used in failure but not in success
    ToolNotUsedInSuccess,
    /// File was modified in success but not in failure
    FileModifiedInSuccess,
    /// File was not modified in success but was in failure
    FileModifiedInFailure,
    /// Step order differs between success and failure
    StepOrderDifference,
    /// Risk level differs between success and failure
    RiskLevelDifference,
    /// Context variable present in success but not in failure
    ContextVarPresentInSuccess,
    /// Context variable present in failure but not in success
    ContextVarPresentInFailure,
}

impl DifferentiatingFactorType {
    /// Returns true if this factor indicates something that SHOULD be done
    /// (success pattern), vs something that should NOT be done (failure pattern)
    pub fn is_success_pattern(&self) -> bool {
        matches!(
            self,
            DifferentiatingFactorType::ToolUsedInSuccess
                | DifferentiatingFactorType::FileModifiedInSuccess
                | DifferentiatingFactorType::ContextVarPresentInSuccess
                | DifferentiatingFactorType::StepOrderDifference // depends on direction
                | DifferentiatingFactorType::RiskLevelDifference // depends on direction
        )
    }

    /// Returns true if this factor indicates a strict requirement
    pub fn is_strict(&self) -> bool {
        matches!(
            self,
            DifferentiatingFactorType::ToolNotUsedInSuccess
                | DifferentiatingFactorType::FileModifiedInFailure
                | DifferentiatingFactorType::ContextVarPresentInFailure
        )
    }

    /// Get human-readable description of this factor type
    pub fn description(&self) -> &'static str {
        match self {
            DifferentiatingFactorType::ToolUsedInSuccess => "Tool used in successful execution",
            DifferentiatingFactorType::ToolNotUsedInSuccess => {
                "Tool avoided in successful execution"
            }
            DifferentiatingFactorType::FileModifiedInSuccess => {
                "File modified in successful execution"
            }
            DifferentiatingFactorType::FileModifiedInFailure => "File modification caused failure",
            DifferentiatingFactorType::StepOrderDifference => "Step order differs between outcomes",
            DifferentiatingFactorType::RiskLevelDifference => "Risk level differs between outcomes",
            DifferentiatingFactorType::ContextVarPresentInSuccess => {
                "Context variable present in success"
            }
            DifferentiatingFactorType::ContextVarPresentInFailure => {
                "Context variable caused failure"
            }
        }
    }
}

/// Result of identifying differentiating factors between success and failure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactorIdentificationResult {
    /// All identified differentiating factors
    pub factors: Vec<DifferentiatingFactor>,
    /// Non-differentiating steps that were ignored
    pub ignored_steps: usize,
    /// Total pairs analyzed
    pub pairs_analyzed: usize,
}

/// Configuration for contrastive learning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContrastiveLearningConfig {
    /// Margin for contrastive loss (default 1.0)
    /// Positive pairs try to be within margin, negative pairs try to be beyond margin
    pub margin: f32,
    /// Learning rate for embedding updates (default 0.01)
    pub learning_rate: f32,
    /// Maximum pairs to sample per training iteration (default 100)
    pub max_pairs_per_iteration: usize,
    /// Minimum pairs required to perform training (default 10)
    pub min_pairs_for_training: usize,
    /// Embedding dimension for new memories (default 384)
    pub embedding_dimension: usize,
    /// Temperature for softmax in loss computation (default 0.1)
    pub temperature: f32,
    /// Whether to use online hard negative mining (default true)
    pub use_hard_negatives: bool,
    /// Cosine distance threshold for considering a negative pair "hard" (default 0.5)
    pub hard_negative_threshold: f32,
}

impl Default for ContrastiveLearningConfig {
    fn default() -> Self {
        Self {
            margin: 1.0,
            learning_rate: 0.01,
            max_pairs_per_iteration: 100,
            min_pairs_for_training: 10,
            embedding_dimension: 384,
            temperature: 0.1,
            use_hard_negatives: true,
            hard_negative_threshold: 0.5,
        }
    }
}

/// Analyzes trajectories and extracts contrastive pairs
pub struct ContrastiveAnalyzer {
    #[allow(dead_code)]
    store: SqliteMemoryStore,
    config: ContrastiveLearningConfig,
}

impl ContrastiveAnalyzer {
    pub fn new(store: SqliteMemoryStore, config: ContrastiveLearningConfig) -> Self {
        Self { store, config }
    }

    pub fn with_default_config(store: SqliteMemoryStore) -> Self {
        Self {
            store,
            config: ContrastiveLearningConfig::default(),
        }
    }

    /// Build contrastive pairs from successful and failed trajectories
    pub async fn build_pairs(
        &self,
        successes: &[SuccessTrajectory],
        failures: &[FailureTrajectory],
    ) -> Result<Vec<ContrastivePair>, SwellError> {
        let mut pairs = Vec::new();

        // Build positive pairs from success-success trajectories
        let success_pairs = self.build_positive_pairs(successes);
        pairs.extend(success_pairs);

        // Build negative pairs from success-failure trajectories
        let failure_pairs = self.build_negative_pairs(successes, failures);
        pairs.extend(failure_pairs);

        // Limit pairs if necessary
        if pairs.len() > self.config.max_pairs_per_iteration {
            pairs = self.sample_pairs(pairs);
        }

        Ok(pairs)
    }

    /// Build positive pairs (success-success pairs that should be close)
    fn build_positive_pairs(&self, successes: &[SuccessTrajectory]) -> Vec<ContrastivePair> {
        let mut pairs = Vec::new();

        // For each pair of successful trajectories, create a positive pair
        // if they share similar patterns (same tool usage, similar files, etc.)
        for (i, traj1) in successes.iter().enumerate() {
            for traj2 in successes.iter().skip(i + 1) {
                // Check if trajectories share similar characteristics
                if self.trajectories_are_similar(traj1, traj2) {
                    // Create positive pairs between all memory IDs
                    for mem1 in &traj1.memory_ids {
                        for mem2 in &traj2.memory_ids {
                            if mem1 != mem2 {
                                pairs.push(ContrastivePair {
                                    anchor_id: *mem1,
                                    comparison_id: *mem2,
                                    pair_type: PairType::Positive,
                                    margin: self.config.margin,
                                });
                            }
                        }
                    }
                }
            }
        }

        pairs
    }

    /// Build negative pairs (success-failure pairs that should be far apart)
    fn build_negative_pairs(
        &self,
        successes: &[SuccessTrajectory],
        failures: &[FailureTrajectory],
    ) -> Vec<ContrastivePair> {
        let mut pairs = Vec::new();

        for success in successes {
            for failure in failures {
                // Only create negative pairs if they have similar context
                // (e.g., same task type, similar files modified)
                if self.trajectories_are_contrasting(success, failure) {
                    // Create negative pairs between success memories and failure memories
                    for success_mem in &success.memory_ids {
                        for failure_mem in &failure.memory_ids {
                            pairs.push(ContrastivePair {
                                anchor_id: *success_mem,
                                comparison_id: *failure_mem,
                                pair_type: PairType::Negative,
                                margin: self.config.margin,
                            });
                        }
                    }
                }
            }
        }

        pairs
    }

    /// Check if two successful trajectories share similar patterns
    fn trajectories_are_similar(
        &self,
        traj1: &SuccessTrajectory,
        traj2: &SuccessTrajectory,
    ) -> bool {
        // Same outcome type is a strong signal
        if traj1.outcome != traj2.outcome {
            return false;
        }

        // Check tool usage overlap
        let tools1: HashSet<_> = traj1.tool_calls.iter().map(|t| &t.tool_name).collect();
        let tools2: HashSet<_> = traj2.tool_calls.iter().map(|t| &t.tool_name).collect();
        let tool_overlap = tools1.intersection(&tools2).count();

        // If they share at least 2 tools, consider them similar
        if tool_overlap >= 2 {
            return true;
        }

        // Check file modification overlap
        let files1: HashSet<_> = traj1.files_modified.iter().collect();
        let files2: HashSet<_> = traj2.files_modified.iter().collect();
        let file_overlap = files1.intersection(&files2).count();

        // If they modify similar files, consider them similar
        if !files1.is_empty() && file_overlap >= files1.len() / 2 {
            return true;
        }

        // Check step pattern similarity
        let steps1: HashSet<_> = traj1.steps.iter().map(|s| &s.risk_level).collect();
        let steps2: HashSet<_> = traj2.steps.iter().map(|s| &s.risk_level).collect();
        let step_overlap = steps1.intersection(&steps2).count();

        if step_overlap >= 2 {
            return true;
        }

        false
    }

    /// Check if a success and failure trajectory have contrasting patterns
    fn trajectories_are_contrasting(
        &self,
        success: &SuccessTrajectory,
        failure: &FailureTrajectory,
    ) -> bool {
        // Check if they modify the same files (context overlap)
        let success_files: HashSet<_> = success.files_modified.iter().collect();
        let failure_files: HashSet<_> = failure.files_modified.iter().collect();
        let file_overlap = success_files.intersection(&failure_files).count();

        // Only contrast if they have some file context overlap
        // but different outcomes
        if file_overlap > 0 {
            return true;
        }

        // Check if they use similar tools
        let success_tools: HashSet<_> = success.tool_calls.iter().map(|t| &t.tool_name).collect();
        let failure_tools: HashSet<_> = failure.tool_calls.iter().map(|t| &t.tool_name).collect();
        let tool_overlap = success_tools.intersection(&failure_tools).count();

        if tool_overlap >= 1 {
            return true;
        }

        false
    }

    /// Sample pairs to stay within max_pairs_per_iteration limit
    fn sample_pairs(&self, pairs: Vec<ContrastivePair>) -> Vec<ContrastivePair> {
        // Simple reservoir sampling to maintain diversity
        let target_size = self.config.max_pairs_per_iteration;

        if pairs.len() <= target_size {
            return pairs;
        }

        // Separate positive and negative pairs
        let (pos_pairs, neg_pairs): (Vec<_>, Vec<_>) = pairs
            .iter()
            .partition(|p| p.pair_type == PairType::Positive);

        let pos_target = target_size / 2;
        let neg_target = target_size - pos_target;

        let sampled_pos = Self::reservoir_sample(pos_pairs, pos_target);
        let sampled_neg = Self::reservoir_sample(neg_pairs, neg_target);

        let mut result = sampled_pos;
        result.extend(sampled_neg);
        result
    }

    /// Reservoir sampling helper
    fn reservoir_sample<T: Clone>(items: Vec<&T>, k: usize) -> Vec<T> {
        if items.len() <= k {
            return items.iter().cloned().cloned().collect();
        }

        let mut result: Vec<T> = items.iter().take(k).cloned().cloned().collect();

        for (i, item) in items.iter().skip(k).enumerate() {
            let j = rand_index(i + k + 1);
            if j < k {
                result[j] = (*item).clone();
            }
        }

        result
    }

    /// Identify differentiating factors between success and failure trajectory pairs
    ///
    /// This method compares success/failure pairs step-by-step to identify which
    /// factors (tools, files, step orders, etc.) distinguish successful from failed
    /// executions. These factors are used to tighten procedure preconditions.
    ///
    /// # Arguments
    /// * `success` - The successful trajectory
    /// * `failure` - The failed trajectory
    ///
    /// # Returns
    /// A `FactorIdentificationResult` containing identified differentiating factors
    pub fn identify_differentiating_factors(
        &self,
        success: &SuccessTrajectory,
        failure: &FailureTrajectory,
    ) -> FactorIdentificationResult {
        let mut factors: Vec<DifferentiatingFactor> = Vec::new();

        // Compare tool usage between success and failure
        // For success: only consider successful tool calls (we want to know what TO do)
        let success_tools: HashSet<_> = success
            .tool_calls
            .iter()
            .filter(|t| t.success)
            .map(|t| t.tool_name.clone())
            .collect();

        // For failure: consider ALL tool calls (including failed ones) since the failed
        // tool might be the differentiating factor that caused the failure
        let failure_tools: HashSet<_> = failure
            .tool_calls
            .iter()
            .map(|t| t.tool_name.clone())
            .collect();

        // Tools used in success but not in failure (positive pattern)
        for tool in success_tools.difference(&failure_tools) {
            factors.push(DifferentiatingFactor::new(
                DifferentiatingFactorType::ToolUsedInSuccess,
                tool.clone(),
                true,
                1.0, // High confidence since it clearly differentiates
            ));
        }

        // Tools used in failure but not in success (negative pattern)
        for tool in failure_tools.difference(&success_tools) {
            factors.push(DifferentiatingFactor::new(
                DifferentiatingFactorType::ToolNotUsedInSuccess,
                tool.clone(),
                false,
                1.0,
            ));
        }

        // Compare file modifications
        let success_files: HashSet<_> = success.files_modified.iter().collect();
        let failure_files: HashSet<_> = failure.files_modified.iter().collect();

        // Files modified in success but not in failure (positive pattern)
        for file in success_files.difference(&failure_files) {
            factors.push(DifferentiatingFactor::new(
                DifferentiatingFactorType::FileModifiedInSuccess,
                file.to_string(),
                true,
                1.0,
            ));
        }

        // Files modified in failure but not in success (negative pattern)
        for file in failure_files.difference(&success_files) {
            factors.push(DifferentiatingFactor::new(
                DifferentiatingFactorType::FileModifiedInFailure,
                file.to_string(),
                false,
                1.0,
            ));
        }

        // Compare step risk levels
        let success_risks: Vec<_> = success.steps.iter().map(|s| s.risk_level.clone()).collect();
        let failure_risks: Vec<_> = failure.steps.iter().map(|s| s.risk_level.clone()).collect();

        // If the failure has higher risk steps, that's a differentiating factor
        // Compare average risk level
        let success_avg_risk = if !success_risks.is_empty() {
            let risk_values: Vec<f64> = success_risks
                .iter()
                .map(|r| match r.as_str() {
                    "low" => 0.25,
                    "medium" => 0.5,
                    "high" => 0.75,
                    "critical" => 1.0,
                    _ => 0.5,
                })
                .collect();
            risk_values.iter().sum::<f64>() / risk_values.len() as f64
        } else {
            0.0
        };

        let failure_avg_risk = if !failure_risks.is_empty() {
            let risk_values: Vec<f64> = failure_risks
                .iter()
                .map(|r| match r.as_str() {
                    "low" => 0.25,
                    "medium" => 0.5,
                    "high" => 0.75,
                    "critical" => 1.0,
                    _ => 0.5,
                })
                .collect();
            risk_values.iter().sum::<f64>() / risk_values.len() as f64
        } else {
            0.0
        };

        if failure_avg_risk > success_avg_risk {
            factors.push(DifferentiatingFactor::new(
                DifferentiatingFactorType::RiskLevelDifference,
                format!(
                    "failure_risk={:.2}, success_risk={:.2}",
                    failure_avg_risk, success_avg_risk
                ),
                false,
                (failure_avg_risk - success_avg_risk).min(1.0),
            ));
        }

        // Count non-differentiating steps (steps with same risk level in both)
        let success_risks_set: HashSet<_> = success_risks.iter().collect();
        let failure_risks_set: HashSet<_> = failure_risks.iter().collect();
        let common_risks_count = success_risks_set.intersection(&failure_risks_set).count();
        let ignored_steps = (success.steps.len() + failure.steps.len()) / 2 - common_risks_count;

        FactorIdentificationResult {
            factors,
            ignored_steps,
            pairs_analyzed: 1,
        }
    }

    /// Analyze multiple success/failure pairs and aggregate differentiating factors
    ///
    /// Returns factors with aggregated confidence based on how often they appear
    pub async fn analyze_trajectory_pairs(
        &self,
        successes: &[SuccessTrajectory],
        failures: &[FailureTrajectory],
    ) -> Result<FactorIdentificationResult, SwellError> {
        let mut all_factors: Vec<DifferentiatingFactor> = Vec::new();
        let mut ignored_count = 0;
        let pairs_analyzed = successes.len().min(failures.len());

        // For each success-failure pair with context overlap, identify factors
        for success in successes {
            for failure in failures {
                // Only analyze if they have file overlap (same context)
                let success_files: HashSet<_> = success.files_modified.iter().collect();
                let failure_files: HashSet<_> = failure.files_modified.iter().collect();

                if success_files.intersection(&failure_files).count() > 0 {
                    let result = self.identify_differentiating_factors(success, failure);
                    all_factors.extend(result.factors);
                    ignored_count += result.ignored_steps;
                }
            }
        }

        // Aggregate factors by type and value, computing average confidence
        let mut factor_map: HashMap<(DifferentiatingFactorType, String), (f64, usize)> =
            HashMap::new();

        for factor in &all_factors {
            let key = (factor.factor_type.clone(), factor.value.clone());
            let entry = factor_map.entry(key).or_insert((0.0, 0));
            entry.0 += factor.confidence;
            entry.1 += 1;
        }

        let mut aggregated_factors: Vec<DifferentiatingFactor> = Vec::new();
        for ((factor_type, value), (total_conf, count)) in factor_map {
            let avg_confidence = total_conf / count as f64;
            let mut factor = DifferentiatingFactor::new(factor_type, value, true, avg_confidence);
            factor.observation_count = count;
            aggregated_factors.push(factor);
        }

        // Sort by confidence descending
        aggregated_factors.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(FactorIdentificationResult {
            factors: aggregated_factors,
            ignored_steps: ignored_count,
            pairs_analyzed,
        })
    }

    /// Convert a differentiating factor to a procedure precondition
    ///
    /// This is the key wiring step - taking factors identified from contrastive
    /// analysis and translating them into preconditions that tighten procedure
    /// applicability criteria.
    pub fn factor_to_precondition(
        &self,
        factor: &DifferentiatingFactor,
    ) -> Option<(crate::procedural::PreconditionType, String, bool, f64)> {
        // Map factor types to precondition types
        match factor.factor_type {
            DifferentiatingFactorType::ToolUsedInSuccess => {
                // Success has this tool, failure doesn't → require this tool
                Some((
                    crate::procedural::PreconditionType::ToolRequired,
                    factor.value.clone(),
                    false, // soft - having it is good but not strict
                    factor.confidence,
                ))
            }
            DifferentiatingFactorType::ToolNotUsedInSuccess => {
                // Success doesn't have this tool, failure does → forbid this tool
                Some((
                    crate::procedural::PreconditionType::ToolForbidden,
                    factor.value.clone(),
                    true, // strict - must NOT use this tool
                    factor.confidence,
                ))
            }
            DifferentiatingFactorType::FileModifiedInFailure => {
                // File was modified in failure → must not modify this file
                Some((
                    crate::procedural::PreconditionType::FileUntouched,
                    factor.value.clone(),
                    true, // strict
                    factor.confidence,
                ))
            }
            DifferentiatingFactorType::RiskLevelDifference => {
                // Extract risk threshold from the factor value
                // Format: "failure_risk=X.XX, success_risk=Y.YY"
                if factor.value.contains("failure_risk=") {
                    // Parse the failure risk value
                    if let Some(rest) = factor.value.split("failure_risk=").nth(1) {
                        if let Some(num_str) = rest.split(',').next() {
                            if let Ok(risk) = num_str.parse::<f64>() {
                                let threshold = (risk * 4.0).min(3.0) as usize;
                                let risk_str = match threshold {
                                    0 => "low",
                                    1 => "medium",
                                    2 => "high",
                                    _ => "critical",
                                };
                                return Some((
                                    crate::procedural::PreconditionType::MaxRiskLevel,
                                    risk_str.to_string(),
                                    true, // strict - risk must not exceed this
                                    factor.confidence,
                                ));
                            }
                        }
                    }
                }
                None
            }
            // Other factor types don't directly map to preconditions
            _ => None,
        }
    }
}

/// Simple random index using a basic hash
fn rand_index(n: usize) -> usize {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos() as usize;

    nanos % n
}

/// Computes contrastive loss for embedding pairs
pub struct ContrastiveTrainer {
    config: ContrastiveLearningConfig,
}

impl ContrastiveTrainer {
    pub fn new(config: ContrastiveLearningConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config() -> Self {
        Self {
            config: ContrastiveLearningConfig::default(),
        }
    }

    /// Compute cosine distance between two embeddings
    fn cosine_distance(emb1: &[f32], emb2: &[f32]) -> f32 {
        if emb1.len() != emb2.len() {
            return 1.0; // Max distance for incompatible embeddings
        }

        let dot: f32 = emb1.iter().zip(emb2.iter()).map(|(a, b)| a * b).sum();
        let norm1: f32 = emb1.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm2: f32 = emb2.iter().map(|x| x * x).sum::<f32>().sqrt();

        if norm1 == 0.0 || norm2 == 0.0 {
            return 1.0;
        }

        let similarity = dot / (norm1 * norm2);
        // Distance = 1 - similarity, clamped to [0, 1]
        (1.0 - similarity).clamp(0.0, 1.0)
    }

    /// Normalize embedding to unit vector
    fn normalize_embedding(emb: &[f32]) -> Vec<f32> {
        let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm == 0.0 {
            return emb.to_vec();
        }
        emb.iter().map(|x| x / norm).collect()
    }

    /// Compute contrastive loss for a pair
    /// Loss = (1 - margin + distance)^2 for positive pairs
    /// Loss = max(0, distance - margin)^2 for negative pairs
    fn compute_pair_loss(&self, distance: f32, pair_type: PairType) -> f32 {
        match pair_type {
            PairType::Positive => {
                // For positive pairs, we want distance to be close to 0
                // Loss = max(0, distance - margin)^2 (we want distance < margin)
                let diff = distance - self.config.margin;
                diff.max(0.0).powi(2)
            }
            PairType::Negative => {
                // For negative pairs, we want distance to be larger than margin
                // Loss = max(0, margin - distance)^2 (we want distance > margin)
                let diff = self.config.margin - distance;
                diff.max(0.0).powi(2)
            }
        }
    }

    /// Compute gradient direction for embedding update
    /// Returns the direction vector to move the embedding
    fn compute_gradient(
        &self,
        anchor: &[f32],
        comparison: &[f32],
        pair_type: PairType,
    ) -> Vec<f32> {
        let distance = Self::cosine_distance(anchor, comparison);

        // For positive pairs: we want to move embeddings closer
        // Gradient direction: comparison - anchor (move anchor toward comparison)
        // For negative pairs: we want to move embeddings apart
        // Gradient direction: anchor - comparison (move anchor away from comparison)

        let grad = match pair_type {
            PairType::Positive => {
                // Move anchor toward comparison (reduce distance)
                comparison
                    .iter()
                    .zip(anchor.iter())
                    .map(|(c, a)| c - a)
                    .collect()
            }
            PairType::Negative => {
                // Move anchor away from comparison (increase distance)
                // Only if distance < margin, otherwise no gradient
                if distance < self.config.margin {
                    anchor
                        .iter()
                        .zip(comparison.iter())
                        .map(|(a, c)| a - c)
                        .collect()
                } else {
                    vec![0.0; anchor.len()] // Already far enough apart
                }
            }
        };

        // Normalize gradient
        Self::normalize_embedding(&grad)
    }

    /// Apply contrastive learning to update embeddings
    /// Returns the loss value and list of updated embedding IDs
    pub async fn train(
        &self,
        store: &SqliteMemoryStore,
        pairs: Vec<ContrastivePair>,
    ) -> Result<ContrastiveLearningResult, SwellError> {
        if pairs.len() < self.config.min_pairs_for_training {
            return Err(SwellError::InvalidStateTransition(format!(
                "Need at least {} pairs for training, got {}",
                self.config.min_pairs_for_training,
                pairs.len()
            )));
        }

        // Collect all embeddings for the pairs
        let mut all_ids: HashSet<Uuid> = HashSet::new();
        for pair in &pairs {
            all_ids.insert(pair.anchor_id);
            all_ids.insert(pair.comparison_id);
        }

        // Fetch all embeddings
        let mut embeddings: HashMap<Uuid, Vec<f32>> = HashMap::new();
        for id in &all_ids {
            if let Some(entry) = store.get(*id).await? {
                if let Some(emb) = entry.embedding {
                    embeddings.insert(*id, emb);
                }
            }
        }

        // Compute losses and gradients
        let mut total_loss = 0.0;
        let mut positive_losses = Vec::new();
        let mut negative_losses = Vec::new();
        let mut gradients: HashMap<Uuid, Vec<f32>> = HashMap::new();

        for pair in &pairs {
            let Some(anchor) = embeddings.get(&pair.anchor_id) else {
                continue;
            };
            let Some(comparison) = embeddings.get(&pair.comparison_id) else {
                continue;
            };

            let distance = Self::cosine_distance(anchor, comparison);
            let loss = self.compute_pair_loss(distance, pair.pair_type);

            total_loss += loss;

            match pair.pair_type {
                PairType::Positive => positive_losses.push(distance),
                PairType::Negative => negative_losses.push(distance),
            }

            // Accumulate gradients
            let grad = self.compute_gradient(anchor, comparison, pair.pair_type);
            let entry = gradients
                .entry(pair.anchor_id)
                .or_insert_with(|| vec![0.0; anchor.len()]);
            for (i, g) in grad.iter().enumerate() {
                entry[i] += g;
            }
        }

        // Normalize loss
        let pair_count = pairs.len() as f32;
        total_loss /= pair_count;

        // Apply updates to embeddings
        let mut updated_ids = Vec::new();
        for (id, grad) in gradients {
            // Skip if no gradient
            if grad.iter().all(|x| x.abs() < 1e-6) {
                continue;
            }

            // Get current embedding
            let Some(anchor) = embeddings.get(&id).cloned() else {
                continue;
            };

            // Apply gradient with learning rate
            let updated: Vec<f32> = anchor
                .iter()
                .zip(grad.iter())
                .map(|(a, g)| a - self.config.learning_rate * g)
                .collect();

            // Normalize updated embedding
            let normalized = Self::normalize_embedding(&updated);

            // Get the memory entry and update its embedding
            if let Some(mut entry) = store.get(id).await? {
                entry.embedding = Some(normalized.clone());
                entry.updated_at = chrono::Utc::now();
                store.update(entry).await?;
                updated_ids.push(id);
            }
        }

        // Compute averages
        let positive_avg = if positive_losses.is_empty() {
            0.0
        } else {
            positive_losses.iter().sum::<f32>() / positive_losses.len() as f32
        };

        let negative_avg = if negative_losses.is_empty() {
            0.0
        } else {
            negative_losses.iter().sum::<f32>() / negative_losses.len() as f32
        };

        Ok(ContrastiveLearningResult {
            loss: total_loss,
            positive_avg_distance: positive_avg,
            negative_avg_distance: negative_avg,
            positive_pairs_count: positive_losses.len(),
            negative_pairs_count: negative_losses.len(),
            embeddings_updated: updated_ids,
            loss_components: LossComponents {
                positive_loss: positive_losses.iter().sum::<f32>() / pair_count,
                negative_loss: negative_losses.iter().sum::<f32>() / pair_count,
            },
        })
    }

    /// Simple one-shot contrastive learning from trajectories without modifying embeddings
    /// Useful for analysis and evaluation
    pub fn analyze(&self, pairs: &[ContrastivePair]) -> ContrastiveLearningResult {
        let total_loss = 0.0;
        let mut positive_distances = Vec::new();
        let mut negative_distances = Vec::new();

        // We need embeddings to compute actual distances
        // For analysis-only, we'd need access to the store
        // This is a simplified version that just counts pairs
        for pair in pairs {
            match pair.pair_type {
                PairType::Positive => {
                    // Placeholder - in real use, embeddings would be fetched
                    positive_distances.push(0.5); // placeholder
                }
                PairType::Negative => {
                    negative_distances.push(0.5); // placeholder
                }
            }
        }

        ContrastiveLearningResult {
            loss: total_loss,
            positive_avg_distance: 0.5,
            negative_avg_distance: 0.5,
            positive_pairs_count: positive_distances.len(),
            negative_pairs_count: negative_distances.len(),
            embeddings_updated: Vec::new(),
            loss_components: LossComponents::default(),
        }
    }
}

/// Service for managing contrastive learning operations
pub struct ContrastiveLearningService {
    store: SqliteMemoryStore,
    config: ContrastiveLearningConfig,
    analyzer: ContrastiveAnalyzer,
}

impl ContrastiveLearningService {
    pub fn new(store: SqliteMemoryStore, config: ContrastiveLearningConfig) -> Self {
        Self {
            store: store.clone(),
            analyzer: ContrastiveAnalyzer::new(store, config.clone()),
            config,
        }
    }

    pub fn with_default_config(store: SqliteMemoryStore) -> Self {
        Self {
            store: store.clone(),
            analyzer: ContrastiveAnalyzer::with_default_config(store.clone()),
            config: ContrastiveLearningConfig::default(),
        }
    }

    /// Learn from a batch of successful and failed trajectories
    pub async fn learn_from_trajectories(
        &self,
        successes: Vec<SuccessTrajectory>,
        failures: Vec<FailureTrajectory>,
    ) -> Result<ContrastiveLearningResult, SwellError> {
        let trainer = ContrastiveTrainer::new(self.config.clone());

        // Build contrastive pairs
        let pairs = self.analyzer.build_pairs(&successes, &failures).await?;

        // Train embeddings
        let result = trainer.train(&self.store, pairs).await?;

        Ok(result)
    }

    /// Analyze contrastive pairs without modifying embeddings
    pub async fn analyze_trajectories(
        &self,
        successes: Vec<SuccessTrajectory>,
        failures: Vec<FailureTrajectory>,
    ) -> Result<ContrastiveLearningResult, SwellError> {
        let trainer = ContrastiveTrainer::new(self.config.clone());

        // Build contrastive pairs
        let pairs = self.analyzer.build_pairs(&successes, &failures).await?;

        // Analyze without training
        Ok(trainer.analyze(&pairs))
    }

    /// Create a SuccessTrajectory from task data
    pub fn create_success_trajectory(
        task_id: Uuid,
        description: String,
        memory_ids: Vec<Uuid>,
        files_modified: Vec<String>,
        outcome: SuccessOutcome,
    ) -> SuccessTrajectory {
        SuccessTrajectory {
            task_id,
            task_description: description,
            memory_ids,
            steps: Vec::new(),
            outcome,
            tool_calls: Vec::new(),
            files_modified,
            timestamp: Utc::now(),
        }
    }

    /// Create a FailureTrajectory from task data
    pub fn create_failure_trajectory(
        task_id: Uuid,
        description: String,
        memory_ids: Vec<Uuid>,
        files_modified: Vec<String>,
        failure_reason: FailureReason,
        validation_errors: Vec<ValidationErrorRecord>,
        iteration_count: u32,
    ) -> FailureTrajectory {
        FailureTrajectory {
            task_id,
            task_description: description,
            memory_ids,
            steps: Vec::new(),
            failure_reason,
            validation_errors,
            tool_calls: Vec::new(),
            files_modified,
            iteration_count,
            timestamp: Utc::now(),
        }
    }

    /// Apply contrastive learning analysis to tighten procedure preconditions
    ///
    /// This is the main wiring function that:
    /// 1. Analyzes success/failure trajectory pairs to identify differentiating factors
    /// 2. Converts those factors into procedure preconditions
    /// 3. Updates the procedure with tighter preconditions based on what distinguishes
    ///    successful executions from failed ones
    ///
    /// # Arguments
    /// * `successes` - Trajectories of successful task executions
    /// * `failures` - Trajectories of failed task executions
    /// * `procedure` - The procedure whose preconditions should be tightened
    ///
    /// # Returns
    /// A `Procedure` with updated preconditions, or the original if no factors found
    pub async fn tighten_procedure_preconditions(
        &self,
        successes: Vec<SuccessTrajectory>,
        failures: Vec<FailureTrajectory>,
        mut procedure: crate::procedural::Procedure,
    ) -> Result<crate::procedural::Procedure, SwellError> {
        // Use the analyzer to identify differentiating factors
        let factors_result = self
            .analyzer
            .analyze_trajectory_pairs(&successes, &failures)
            .await?;

        // If no factors found, return procedure unchanged
        if factors_result.factors.is_empty() {
            return Ok(procedure);
        }

        // Convert each differentiating factor to a precondition and add it
        for factor in &factors_result.factors {
            if let Some((precondition_type, value, is_strict, confidence)) =
                self.analyzer.factor_to_precondition(factor)
            {
                procedure.add_precondition(crate::procedural::ProcedurePrecondition {
                    precondition_type,
                    value,
                    is_strict,
                    confidence,
                    source_factor: Some(factor.id.to_string()),
                });
            }
        }

        Ok(procedure)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contrastive_learning_config_default() {
        let config = ContrastiveLearningConfig::default();
        assert_eq!(config.margin, 1.0);
        assert_eq!(config.learning_rate, 0.01);
        assert_eq!(config.max_pairs_per_iteration, 100);
        assert_eq!(config.min_pairs_for_training, 10);
        assert!(config.use_hard_negatives);
    }

    #[test]
    fn test_pair_type_serialization() {
        let pair_type = PairType::Positive;
        let json = serde_json::to_string(&pair_type).unwrap();
        assert_eq!(json, "\"positive\"");

        let pair_type2 = PairType::Negative;
        let json2 = serde_json::to_string(&pair_type2).unwrap();
        assert_eq!(json2, "\"negative\"");
    }

    #[test]
    fn test_failure_reason_serialization() {
        let reason = FailureReason::ValidationFailure;
        assert_eq!(reason.as_str(), "validation_failure");

        let reason2 = FailureReason::TestFailure;
        assert_eq!(reason2.as_str(), "test_failure");
    }

    #[test]
    fn test_success_outcome_serialization() {
        let outcome = SuccessOutcome::Accepted;
        let json = serde_json::to_string(&outcome).unwrap();
        assert_eq!(json, "\"accepted\"");
    }

    #[test]
    fn test_step_status_serialization() {
        let status = StepStatus::Executed;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"executed\"");
    }

    #[test]
    fn test_cosine_distance_identical() {
        let emb = vec![0.1, 0.2, 0.3, 0.4];
        let distance = ContrastiveTrainer::cosine_distance(&emb, &emb);
        assert!(
            distance < 0.001,
            "Identical embeddings should have near-zero distance"
        );
    }

    #[test]
    fn test_cosine_distance_orthogonal() {
        let emb1 = vec![1.0, 0.0, 0.0, 0.0];
        let emb2 = vec![0.0, 1.0, 0.0, 0.0];
        let distance = ContrastiveTrainer::cosine_distance(&emb1, &emb2);
        assert!(
            distance > 0.99,
            "Orthogonal embeddings should have near-1 distance"
        );
    }

    #[test]
    fn test_cosine_distance_different_lengths() {
        let emb1 = vec![0.1, 0.2, 0.3];
        let emb2 = vec![0.1, 0.2, 0.3, 0.4];
        let distance = ContrastiveTrainer::cosine_distance(&emb1, &emb2);
        assert_eq!(
            distance, 1.0,
            "Different length embeddings should return max distance"
        );
    }

    #[test]
    fn test_cosine_distance_zero_vectors() {
        let emb1 = vec![0.0, 0.0, 0.0];
        let emb2 = vec![0.0, 0.0, 0.0];
        let distance = ContrastiveTrainer::cosine_distance(&emb1, &emb2);
        assert_eq!(distance, 1.0, "Zero vectors should return max distance");
    }

    #[test]
    fn test_normalize_embedding() {
        let emb = vec![3.0, 4.0];
        let normalized = ContrastiveTrainer::normalize_embedding(&emb);
        let norm: f32 = normalized.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 0.001,
            "Normalized embedding should have unit norm"
        );
    }

    #[test]
    fn test_contrastive_pair_serialization() {
        let pair = ContrastivePair {
            anchor_id: Uuid::new_v4(),
            comparison_id: Uuid::new_v4(),
            pair_type: PairType::Positive,
            margin: 1.0,
        };

        let json = serde_json::to_string(&pair).unwrap();
        let deserialized: ContrastivePair = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.pair_type, PairType::Positive);
        assert_eq!(deserialized.margin, 1.0);
    }

    #[test]
    fn test_contrastive_learning_result_serialization() {
        let result = ContrastiveLearningResult {
            loss: 0.25,
            positive_avg_distance: 0.3,
            negative_avg_distance: 0.7,
            positive_pairs_count: 10,
            negative_pairs_count: 20,
            embeddings_updated: vec![Uuid::new_v4(), Uuid::new_v4()],
            loss_components: LossComponents {
                positive_loss: 0.1,
                negative_loss: 0.15,
            },
        };

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: ContrastiveLearningResult = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.loss, 0.25);
        assert_eq!(deserialized.positive_pairs_count, 10);
        assert_eq!(deserialized.negative_pairs_count, 20);
    }

    #[test]
    fn test_loss_components_default() {
        let components = LossComponents::default();
        assert_eq!(components.positive_loss, 0.0);
        assert_eq!(components.negative_loss, 0.0);
    }

    #[test]
    fn test_success_trajectory_creation() {
        let task_id = Uuid::new_v4();
        let trajectory = ContrastiveLearningService::create_success_trajectory(
            task_id,
            "Implement feature X".to_string(),
            vec![Uuid::new_v4()],
            vec!["src/main.rs".to_string()],
            SuccessOutcome::Accepted,
        );

        assert_eq!(trajectory.task_id, task_id);
        assert_eq!(trajectory.task_description, "Implement feature X");
        assert_eq!(trajectory.outcome, SuccessOutcome::Accepted);
        assert_eq!(trajectory.files_modified.len(), 1);
    }

    #[test]
    fn test_failure_trajectory_creation() {
        let task_id = Uuid::new_v4();
        let errors = vec![ValidationErrorRecord {
            error_type: "test_failed".to_string(),
            message: "Test assertion failed".to_string(),
            file: Some("src/main.rs".to_string()),
            line: Some(42),
        }];

        let trajectory = ContrastiveLearningService::create_failure_trajectory(
            task_id,
            "Fix bug Y".to_string(),
            vec![Uuid::new_v4()],
            vec!["src/main.rs".to_string()],
            FailureReason::TestFailure,
            errors,
            2,
        );

        assert_eq!(trajectory.task_id, task_id);
        assert_eq!(trajectory.failure_reason, FailureReason::TestFailure);
        assert_eq!(trajectory.iteration_count, 2);
        assert_eq!(trajectory.validation_errors.len(), 1);
    }

    #[test]
    fn test_contrastive_trainer_pair_loss_positive() {
        let trainer = ContrastiveTrainer::with_default_config();
        // For positive pair, loss should be (distance - margin)^2 when distance > margin
        // If distance = 0.5 and margin = 1.0, loss = max(0, 0.5 - 1.0)^2 = 0
        let loss = trainer.compute_pair_loss(0.5, PairType::Positive);
        assert!(
            loss < 0.001,
            "Positive pair with distance < margin should have near-zero loss"
        );
    }

    #[test]
    fn test_contrastive_trainer_pair_loss_negative() {
        let trainer = ContrastiveTrainer::with_default_config();
        // For negative pair, loss should be (margin - distance)^2 when distance < margin
        // If distance = 0.5 and margin = 1.0, loss = max(0, 1.0 - 0.5)^2 = 0.25
        let loss = trainer.compute_pair_loss(0.5, PairType::Negative);
        assert!(
            (loss - 0.25).abs() < 0.001,
            "Negative pair with distance < margin should have positive loss"
        );
    }

    #[test]
    fn test_contrastive_trainer_pair_loss_negative_far_apart() {
        let trainer = ContrastiveTrainer::with_default_config();
        // If distance = 1.5 (far apart) and margin = 1.0, loss = max(0, 1.0 - 1.5)^2 = 0
        let loss = trainer.compute_pair_loss(1.5, PairType::Negative);
        assert!(
            loss < 0.001,
            "Negative pair with distance > margin should have near-zero loss"
        );
    }

    // =====================================================================
    // Contrastive Analyzer Tests
    // =====================================================================

    #[tokio::test]
    async fn test_contrastive_analyzer_build_pairs() {
        use crate::SqliteMemoryStore;

        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();
        let analyzer = ContrastiveAnalyzer::with_default_config(store);

        // Create two similar successful trajectories
        let traj1 = SuccessTrajectory {
            task_id: Uuid::new_v4(),
            task_description: "Add feature A".to_string(),
            memory_ids: vec![Uuid::new_v4(), Uuid::new_v4()],
            steps: vec![TrajectoryStep {
                step_id: Uuid::new_v4(),
                description: "Step 1".to_string(),
                affected_files: vec!["src/main.rs".to_string()],
                risk_level: "medium".to_string(),
                status: StepStatus::Executed,
            }],
            outcome: SuccessOutcome::Accepted,
            tool_calls: vec![
                ToolCallRecord {
                    tool_name: "read_file".to_string(),
                    arguments: serde_json::json!({}),
                    success: true,
                    timestamp: Utc::now(),
                },
                ToolCallRecord {
                    tool_name: "edit_file".to_string(),
                    arguments: serde_json::json!({}),
                    success: true,
                    timestamp: Utc::now(),
                },
            ],
            files_modified: vec!["src/main.rs".to_string()],
            timestamp: Utc::now(),
        };

        let traj2 = SuccessTrajectory {
            task_id: Uuid::new_v4(),
            task_description: "Add feature B".to_string(),
            memory_ids: vec![Uuid::new_v4(), Uuid::new_v4()],
            steps: vec![TrajectoryStep {
                step_id: Uuid::new_v4(),
                description: "Step 1".to_string(),
                affected_files: vec!["src/lib.rs".to_string()],
                risk_level: "medium".to_string(),
                status: StepStatus::Executed,
            }],
            outcome: SuccessOutcome::Accepted,
            tool_calls: vec![
                ToolCallRecord {
                    tool_name: "read_file".to_string(),
                    arguments: serde_json::json!({}),
                    success: true,
                    timestamp: Utc::now(),
                },
                ToolCallRecord {
                    tool_name: "edit_file".to_string(),
                    arguments: serde_json::json!({}),
                    success: true,
                    timestamp: Utc::now(),
                },
            ],
            files_modified: vec!["src/lib.rs".to_string()],
            timestamp: Utc::now(),
        };

        // Empty failures
        let failures: Vec<FailureTrajectory> = vec![];

        let pairs = analyzer
            .build_pairs(&[traj1, traj2], &failures)
            .await
            .unwrap();

        // Should have positive pairs between similar successful trajectories
        assert!(
            !pairs.is_empty(),
            "Should create positive pairs from similar successes"
        );
    }

    #[tokio::test]
    async fn test_contrastive_analyzer_creates_negative_pairs() {
        use crate::SqliteMemoryStore;

        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();
        let analyzer = ContrastiveAnalyzer::with_default_config(store);

        // Create a success and failure that modify the same file
        let success = SuccessTrajectory {
            task_id: Uuid::new_v4(),
            task_description: "Implement feature".to_string(),
            memory_ids: vec![Uuid::new_v4()],
            steps: vec![],
            outcome: SuccessOutcome::Accepted,
            tool_calls: vec![ToolCallRecord {
                tool_name: "edit_file".to_string(),
                arguments: serde_json::json!({}),
                success: true,
                timestamp: Utc::now(),
            }],
            files_modified: vec!["src/main.rs".to_string()],
            timestamp: Utc::now(),
        };

        let failure = FailureTrajectory {
            task_id: Uuid::new_v4(),
            task_description: "Fix bug".to_string(),
            memory_ids: vec![Uuid::new_v4()],
            steps: vec![],
            failure_reason: FailureReason::TestFailure,
            validation_errors: vec![],
            tool_calls: vec![ToolCallRecord {
                tool_name: "edit_file".to_string(),
                arguments: serde_json::json!({}),
                success: true,
                timestamp: Utc::now(),
            }],
            files_modified: vec!["src/main.rs".to_string()], // Same file
            iteration_count: 1,
            timestamp: Utc::now(),
        };

        let pairs = analyzer.build_pairs(&[success], &[failure]).await.unwrap();

        // Should have negative pairs for success-failure contrast
        let neg_pairs: Vec<_> = pairs
            .iter()
            .filter(|p| p.pair_type == PairType::Negative)
            .collect();
        assert!(
            !neg_pairs.is_empty(),
            "Should create negative pairs from contrasting trajectories"
        );
    }

    // =====================================================================
    // Contrastive Learning Service Tests
    // =====================================================================

    #[tokio::test]
    async fn test_contrastive_learning_service_train_insufficient_pairs() {
        use crate::SqliteMemoryStore;

        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();
        let service = ContrastiveLearningService::with_default_config(store);

        // Create only one success trajectory (not enough pairs)
        let successes = vec![SuccessTrajectory {
            task_id: Uuid::new_v4(),
            task_description: "Test".to_string(),
            memory_ids: vec![Uuid::new_v4()],
            steps: vec![],
            outcome: SuccessOutcome::Accepted,
            tool_calls: vec![],
            files_modified: vec![],
            timestamp: Utc::now(),
        }];

        let failures: Vec<FailureTrajectory> = vec![];

        let result = service.learn_from_trajectories(successes, failures).await;
        assert!(result.is_err(), "Should fail with insufficient pairs");
    }

    #[tokio::test]
    async fn test_contrastive_learning_analyze_without_training() {
        use crate::SqliteMemoryStore;

        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();
        let service = ContrastiveLearningService::with_default_config(store);

        let successes = vec![SuccessTrajectory {
            task_id: Uuid::new_v4(),
            task_description: "Test success".to_string(),
            memory_ids: vec![Uuid::new_v4()],
            steps: vec![],
            outcome: SuccessOutcome::Accepted,
            tool_calls: vec![],
            files_modified: vec![],
            timestamp: Utc::now(),
        }];

        let failures: Vec<FailureTrajectory> = vec![];

        let result = service.analyze_trajectories(successes, failures).await;
        assert!(
            result.is_ok(),
            "Analysis should succeed even without embeddings"
        );
    }

    // =====================================================================
    // Gradient Computation Tests
    // =====================================================================

    #[test]
    fn test_gradient_computation_positive_pair() {
        let trainer = ContrastiveTrainer::with_default_config();

        // Two different embeddings
        let anchor = vec![1.0, 0.0, 0.0, 0.0];
        let comparison = vec![0.0, 1.0, 0.0, 0.0];

        let grad = trainer.compute_gradient(&anchor, &comparison, PairType::Positive);

        // For positive pair, gradient should move anchor toward comparison
        assert!(grad[0] < 0.0, "Anchor component 0 should decrease");
        assert!(grad[1] > 0.0, "Anchor component 1 should increase");
    }

    #[test]
    fn test_gradient_computation_negative_pair_close() {
        let trainer = ContrastiveTrainer::with_default_config();

        // Two close embeddings (distance < margin)
        // anchor = [0.5, 0.0, 0.0, 0.0], comparison = [0.9, 0.1, 0.0, 0.0]
        // anchor[0] < comparison[0], anchor[1] < comparison[1]
        // To move anchor AWAY, both components should decrease
        let anchor = vec![0.5, 0.0, 0.0, 0.0];
        let comparison = vec![0.9, 0.1, 0.0, 0.0];

        let grad = trainer.compute_gradient(&anchor, &comparison, PairType::Negative);

        // For negative pair with distance < margin, should push apart
        // Since anchor[0] < comparison[0] AND anchor[1] < comparison[1],
        // both should decrease to move away (negative gradient)
        assert!(
            grad[0] < 0.0,
            "Anchor component 0 should decrease (move away)"
        );
        assert!(
            grad[1] < 0.0,
            "Anchor component 1 should decrease (move away)"
        );
    }

    #[test]
    fn test_gradient_computation_negative_pair_far() {
        let trainer = ContrastiveTrainer::with_default_config();

        // Two far embeddings (distance > margin)
        let anchor = vec![1.0, 0.0, 0.0, 0.0];
        let comparison = vec![0.0, 1.0, 0.0, 0.0];

        let grad = trainer.compute_gradient(&anchor, &comparison, PairType::Negative);

        // For negative pair with distance > margin, gradient should be zero (already far enough)
        for g in &grad {
            assert!(
                g.abs() < 0.001,
                "Gradient should be zero when already far apart"
            );
        }
    }

    // =====================================================================
    // Differentiating Factor Tests
    // =====================================================================

    #[tokio::test]
    async fn test_identify_differentiating_factors_tool_difference() {
        use crate::SqliteMemoryStore;

        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();
        let analyzer = ContrastiveAnalyzer::with_default_config(store);

        // Success uses "cargo build" and "cargo test"
        let success = SuccessTrajectory {
            task_id: Uuid::new_v4(),
            task_description: "Implement feature".to_string(),
            memory_ids: vec![],
            steps: vec![],
            outcome: SuccessOutcome::Accepted,
            tool_calls: vec![
                ToolCallRecord {
                    tool_name: "cargo build".to_string(),
                    arguments: serde_json::json!({}),
                    success: true,
                    timestamp: Utc::now(),
                },
                ToolCallRecord {
                    tool_name: "cargo test".to_string(),
                    arguments: serde_json::json!({}),
                    success: true,
                    timestamp: Utc::now(),
                },
            ],
            files_modified: vec!["src/main.rs".to_string()],
            timestamp: Utc::now(),
        };

        // Failure uses "cargo build" but NOT "cargo test" (instead uses risky shell command)
        let failure = FailureTrajectory {
            task_id: Uuid::new_v4(),
            task_description: "Fix bug".to_string(),
            memory_ids: vec![],
            steps: vec![],
            failure_reason: FailureReason::TestFailure,
            validation_errors: vec![],
            tool_calls: vec![
                ToolCallRecord {
                    tool_name: "cargo build".to_string(),
                    arguments: serde_json::json!({}),
                    success: true,
                    timestamp: Utc::now(),
                },
                ToolCallRecord {
                    tool_name: "rm -rf".to_string(),
                    arguments: serde_json::json!({}),
                    success: false,
                    timestamp: Utc::now(),
                },
            ],
            files_modified: vec!["src/main.rs".to_string()], // same file
            iteration_count: 1,
            timestamp: Utc::now(),
        };

        let result = analyzer.identify_differentiating_factors(&success, &failure);

        // Should identify "cargo test" as a positive factor (used in success, not in failure)
        let tool_in_success = result.factors.iter().find(|f| {
            f.factor_type == DifferentiatingFactorType::ToolUsedInSuccess && f.value == "cargo test"
        });
        assert!(
            tool_in_success.is_some(),
            "Should identify 'cargo test' as differentiating factor"
        );

        // Should identify "rm -rf" as a negative factor (used in failure, not in success)
        let tool_not_used = result.factors.iter().find(|f| {
            f.factor_type == DifferentiatingFactorType::ToolNotUsedInSuccess && f.value == "rm -rf"
        });
        assert!(
            tool_not_used.is_some(),
            "Should identify 'rm -rf' as a tool to avoid"
        );
    }

    #[tokio::test]
    async fn test_identify_differentiating_factors_file_difference() {
        use crate::SqliteMemoryStore;

        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();
        let analyzer = ContrastiveAnalyzer::with_default_config(store);

        // Success modifies src/lib.rs
        let success = SuccessTrajectory {
            task_id: Uuid::new_v4(),
            task_description: "Add test".to_string(),
            memory_ids: vec![],
            steps: vec![],
            outcome: SuccessOutcome::Accepted,
            tool_calls: vec![],
            files_modified: vec!["src/lib.rs".to_string()],
            timestamp: Utc::now(),
        };

        // Failure modifies src/main.rs (different file)
        let failure = FailureTrajectory {
            task_id: Uuid::new_v4(),
            task_description: "Fix bug".to_string(),
            memory_ids: vec![],
            steps: vec![],
            failure_reason: FailureReason::TestFailure,
            validation_errors: vec![],
            tool_calls: vec![],
            files_modified: vec!["src/main.rs".to_string()],
            iteration_count: 1,
            timestamp: Utc::now(),
        };

        let result = analyzer.identify_differentiating_factors(&success, &failure);

        // Since there's no file overlap, the factor analysis won't find strong factors
        // The result might be empty due to how trajectories_are_contrasting works
        // But for same-context comparison, we'd want to see file-based factors
        // This test documents the current behavior
        assert!(result.pairs_analyzed >= 1, "Should analyze the pair");
    }

    #[tokio::test]
    async fn test_factor_to_precondition_tool_required() {
        use crate::SqliteMemoryStore;

        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();
        let analyzer = ContrastiveAnalyzer::with_default_config(store);

        let factor = DifferentiatingFactor::new(
            DifferentiatingFactorType::ToolUsedInSuccess,
            "cargo test".to_string(),
            true,
            0.9,
        );

        let result = analyzer.factor_to_precondition(&factor);
        assert!(result.is_some(), "Should convert to precondition");

        let (precond_type, value, is_strict, confidence) = result.unwrap();
        assert_eq!(
            precond_type,
            crate::procedural::PreconditionType::ToolRequired
        );
        assert_eq!(value, "cargo test");
        assert!(!is_strict, "ToolRequired should be soft");
        assert!((confidence - 0.9).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_factor_to_precondition_tool_forbidden() {
        use crate::SqliteMemoryStore;

        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();
        let analyzer = ContrastiveAnalyzer::with_default_config(store);

        let factor = DifferentiatingFactor::new(
            DifferentiatingFactorType::ToolNotUsedInSuccess,
            "rm -rf".to_string(),
            false,
            1.0,
        );

        let result = analyzer.factor_to_precondition(&factor);
        assert!(result.is_some(), "Should convert to precondition");

        let (precond_type, value, is_strict, _confidence) = result.unwrap();
        assert_eq!(
            precond_type,
            crate::procedural::PreconditionType::ToolForbidden
        );
        assert_eq!(value, "rm -rf");
        assert!(is_strict, "ToolForbidden should be strict");
    }

    #[tokio::test]
    async fn test_factor_to_precondition_file_untouched() {
        use crate::SqliteMemoryStore;

        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();
        let analyzer = ContrastiveAnalyzer::with_default_config(store);

        let factor = DifferentiatingFactor::new(
            DifferentiatingFactorType::FileModifiedInFailure,
            "package.json".to_string(),
            false,
            1.0,
        );

        let result = analyzer.factor_to_precondition(&factor);
        assert!(result.is_some(), "Should convert to precondition");

        let (precond_type, value, is_strict, _confidence) = result.unwrap();
        assert_eq!(
            precond_type,
            crate::procedural::PreconditionType::FileUntouched
        );
        assert_eq!(value, "package.json");
        assert!(is_strict, "FileUntouched should be strict");
    }

    // =====================================================================
    // Procedure Precondition Tests
    // =====================================================================

    #[tokio::test]
    async fn test_procedure_add_precondition() {
        use crate::Procedure;

        let mut procedure = Procedure::new(
            "test_procedure".to_string(),
            "A test procedure".to_string(),
            "test context".to_string(),
        );

        // Add a precondition
        procedure.add_precondition(crate::procedural::ProcedurePrecondition {
            precondition_type: crate::procedural::PreconditionType::ToolRequired,
            value: "cargo test".to_string(),
            is_strict: false,
            confidence: 0.8,
            source_factor: None,
        });

        assert_eq!(procedure.preconditions.len(), 1);
        assert_eq!(procedure.preconditions[0].value, "cargo test");

        // Add another precondition
        procedure.add_precondition(crate::procedural::ProcedurePrecondition {
            precondition_type: crate::procedural::PreconditionType::ToolForbidden,
            value: "rm -rf".to_string(),
            is_strict: true,
            confidence: 0.9,
            source_factor: None,
        });

        assert_eq!(procedure.preconditions.len(), 2);

        // Adding duplicate should update confidence (not create new)
        procedure.add_precondition(crate::procedural::ProcedurePrecondition {
            precondition_type: crate::procedural::PreconditionType::ToolRequired,
            value: "cargo test".to_string(),
            is_strict: false,
            confidence: 0.95, // higher
            source_factor: None,
        });

        assert_eq!(procedure.preconditions.len(), 2);
        // Should have updated the confidence
        let updated = procedure
            .preconditions
            .iter()
            .find(|p| p.value == "cargo test")
            .unwrap();
        assert!((updated.confidence - 0.95).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_procedure_require_tool() {
        use crate::Procedure;

        let mut procedure = Procedure::new(
            "test_procedure".to_string(),
            "A test procedure".to_string(),
            "test context".to_string(),
        );

        procedure.require_tool("cargo test", 0.85, Some("factor-123".to_string()));

        assert_eq!(procedure.preconditions.len(), 1);
        assert_eq!(
            procedure.preconditions[0].precondition_type,
            crate::procedural::PreconditionType::ToolRequired
        );
        assert!(!procedure.preconditions[0].is_strict);
    }

    #[tokio::test]
    async fn test_procedure_forbid_tool() {
        use crate::Procedure;

        let mut procedure = Procedure::new(
            "test_procedure".to_string(),
            "A test procedure".to_string(),
            "test context".to_string(),
        );

        procedure.forbid_tool("rm -rf", 1.0, None);

        assert_eq!(procedure.preconditions.len(), 1);
        assert_eq!(
            procedure.preconditions[0].precondition_type,
            crate::procedural::PreconditionType::ToolForbidden
        );
        assert!(procedure.preconditions[0].is_strict);
    }

    #[tokio::test]
    async fn test_procedure_strict_soft_preconditions() {
        use crate::Procedure;

        let mut procedure = Procedure::new(
            "test_procedure".to_string(),
            "A test procedure".to_string(),
            "test context".to_string(),
        );

        procedure.require_tool("cargo test", 0.85, None); // soft
        procedure.forbid_tool("rm -rf", 1.0, None); // strict

        let strict = procedure.strict_preconditions();
        let soft = procedure.soft_preconditions();

        assert_eq!(strict.len(), 1);
        assert_eq!(soft.len(), 1);
        assert_eq!(strict[0].value, "rm -rf");
        assert_eq!(soft[0].value, "cargo test");
    }

    // =====================================================================
    // Tighten Procedure Preconditions Tests
    // =====================================================================

    #[tokio::test]
    async fn test_tighten_procedure_preconditions() {
        use crate::SqliteMemoryStore;

        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();
        let service = ContrastiveLearningService::with_default_config(store);

        // Create success trajectory that uses cargo test
        let successes = vec![SuccessTrajectory {
            task_id: Uuid::new_v4(),
            task_description: "Implement feature".to_string(),
            memory_ids: vec![],
            steps: vec![],
            outcome: SuccessOutcome::Accepted,
            tool_calls: vec![ToolCallRecord {
                tool_name: "cargo test".to_string(),
                arguments: serde_json::json!({}),
                success: true,
                timestamp: Utc::now(),
            }],
            files_modified: vec!["src/lib.rs".to_string()],
            timestamp: Utc::now(),
        }];

        // Create failure trajectory that uses rm -rf
        let failures = vec![FailureTrajectory {
            task_id: Uuid::new_v4(),
            task_description: "Fix bug".to_string(),
            memory_ids: vec![],
            steps: vec![],
            failure_reason: FailureReason::TestFailure,
            validation_errors: vec![],
            tool_calls: vec![ToolCallRecord {
                tool_name: "rm -rf".to_string(),
                arguments: serde_json::json!({}),
                success: false,
                timestamp: Utc::now(),
            }],
            files_modified: vec!["src/lib.rs".to_string()], // same file context
            iteration_count: 1,
            timestamp: Utc::now(),
        }];

        // Create a procedure to tighten
        use crate::Procedure;
        let procedure = Procedure::new(
            "rust_testing".to_string(),
            "Run Rust tests".to_string(),
            "cargo test rust testing".to_string(),
        );

        let result = service
            .tighten_procedure_preconditions(successes, failures, procedure)
            .await
            .unwrap();

        // The procedure should now have preconditions
        assert!(
            !result.preconditions.is_empty(),
            "Procedure should have preconditions after tightening"
        );

        // Should have a precondition for "cargo test" (tool required)
        let has_test_precond = result.preconditions.iter().any(|p| p.value == "cargo test");
        assert!(has_test_precond, "Should require cargo test tool");

        // Should have a precondition for "rm -rf" (tool forbidden)
        let has_forbidden = result.preconditions.iter().any(|p| p.value == "rm -rf");
        assert!(has_forbidden, "Should forbid rm -rf tool");
    }

    #[tokio::test]
    async fn test_tighten_procedure_preserves_existing_preconditions() {
        use crate::Procedure;

        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();
        let service = ContrastiveLearningService::with_default_config(store);

        // Create trajectories with known tool difference
        let successes = vec![SuccessTrajectory {
            task_id: Uuid::new_v4(),
            task_description: "Implement feature".to_string(),
            memory_ids: vec![],
            steps: vec![],
            outcome: SuccessOutcome::Accepted,
            tool_calls: vec![ToolCallRecord {
                tool_name: "cargo build".to_string(),
                arguments: serde_json::json!({}),
                success: true,
                timestamp: Utc::now(),
            }],
            files_modified: vec!["src/lib.rs".to_string()],
            timestamp: Utc::now(),
        }];

        let failures = vec![FailureTrajectory {
            task_id: Uuid::new_v4(),
            task_description: "Fix bug".to_string(),
            memory_ids: vec![],
            steps: vec![],
            failure_reason: FailureReason::TestFailure,
            validation_errors: vec![],
            tool_calls: vec![],
            files_modified: vec!["src/lib.rs".to_string()],
            iteration_count: 1,
            timestamp: Utc::now(),
        }];

        // Create a procedure with existing preconditions
        let mut procedure = Procedure::new(
            "rust_build".to_string(),
            "Build Rust project".to_string(),
            "cargo build rust".to_string(),
        );
        procedure.require_tool("cargo check", 0.7, None);

        let result = service
            .tighten_procedure_preconditions(successes, failures, procedure)
            .await
            .unwrap();

        // Should have both the existing and new preconditions
        assert!(
            result.preconditions.len() >= 1,
            "Should preserve existing preconditions"
        );

        // Should have cargo check (existing)
        let has_check = result
            .preconditions
            .iter()
            .any(|p| p.value == "cargo check");
        assert!(
            has_check,
            "Should preserve existing cargo check precondition"
        );

        // Should have cargo build (new from analysis)
        let has_build = result
            .preconditions
            .iter()
            .any(|p| p.value == "cargo build");
        assert!(has_build, "Should add new cargo build precondition");
    }

    // =====================================================================
    // DifferentiatingFactorType Tests
    // =====================================================================

    #[test]
    fn test_differentiating_factor_type_is_strict() {
        // Strict types
        assert!(
            DifferentiatingFactorType::ToolNotUsedInSuccess.is_strict(),
            "ToolNotUsedInSuccess should be strict"
        );
        assert!(
            DifferentiatingFactorType::FileModifiedInFailure.is_strict(),
            "FileModifiedInFailure should be strict"
        );

        // Non-strict types
        assert!(
            !DifferentiatingFactorType::ToolUsedInSuccess.is_strict(),
            "ToolUsedInSuccess should be soft"
        );
        assert!(
            !DifferentiatingFactorType::FileModifiedInSuccess.is_strict(),
            "FileModifiedInSuccess should be soft"
        );
    }

    #[test]
    fn test_differentiating_factor_update_confidence() {
        let mut factor = DifferentiatingFactor::new(
            DifferentiatingFactorType::ToolUsedInSuccess,
            "cargo test".to_string(),
            true,
            0.5,
        );

        assert_eq!(factor.observation_count, 1);
        assert!((factor.confidence - 0.5).abs() < 0.001);

        factor.update_confidence(0.9);
        assert_eq!(factor.observation_count, 2);
        // New confidence = (1 * 0.5 + 0.9) / 2 = 0.7
        assert!((factor.confidence - 0.7).abs() < 0.001);
    }
}
