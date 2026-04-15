// learning_pipeline.rs - 5-Stage Learning Pipeline
//
// This module implements the 5-stage learning pipeline for memory entries:
// 1. Observation - Raw event captured (entry first observed)
// 2. Pattern Detection - Recurring sequences identified (requires N≥3 occurrences)
// 3. Hypothesis - Proposed rule (requires N≥5 with >60% success rate)
// 4. Evidence Accumulation - Count successes/failures (requires N≥10 with >80% success rate)
// 5. Promotion or Deprecation - Promoted to knowledge or deprecated
//
// Entries failing threshold requirements at any stage are deprecated and deprioritized.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Stage in the 5-stage learning pipeline
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineStage {
    /// Stage 1: Raw observation captured - entry first seen
    Observation,
    /// Stage 2: Pattern detected - recurring sequences identified (N≥3 occurrences)
    Pattern,
    /// Stage 3: Hypothesis formed - proposed rule with initial validation (N≥5, success >60%)
    Hypothesis,
    /// Stage 4: Evidence accumulating - successes/failures tracked (N≥10, success >80%)
    Evidence,
    /// Stage 5a: Promoted - high confidence knowledge entry
    Promoted,
    /// Stage 5b: Deprecated - failed to meet thresholds, deprioritized
    Deprecated,
}

impl PipelineStage {
    /// Get the next stage in the pipeline
    pub fn next_stage(&self) -> Option<PipelineStage> {
        match self {
            PipelineStage::Observation => Some(PipelineStage::Pattern),
            PipelineStage::Pattern => Some(PipelineStage::Hypothesis),
            PipelineStage::Hypothesis => Some(PipelineStage::Evidence),
            PipelineStage::Evidence => Some(PipelineStage::Promoted),
            PipelineStage::Promoted | PipelineStage::Deprecated => None,
        }
    }

    /// Check if this stage is a terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(self, PipelineStage::Promoted | PipelineStage::Deprecated)
    }

    /// Human-readable name for the stage
    pub fn name(&self) -> &'static str {
        match self {
            PipelineStage::Observation => "Observation",
            PipelineStage::Pattern => "Pattern Detection",
            PipelineStage::Hypothesis => "Hypothesis",
            PipelineStage::Evidence => "Evidence Accumulation",
            PipelineStage::Promoted => "Promoted",
            PipelineStage::Deprecated => "Deprecated",
        }
    }
}

impl std::fmt::Display for PipelineStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Configurable thresholds for the learning pipeline stages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineThresholds {
    /// Pattern detection: minimum occurrences required
    pub pattern_min_occurrences: u32,
    /// Hypothesis: minimum occurrences required
    pub hypothesis_min_occurrences: u32,
    /// Hypothesis: minimum success rate (0.0 to 1.0)
    pub hypothesis_min_success_rate: f64,
    /// Evidence: minimum occurrences required
    pub evidence_min_occurrences: u32,
    /// Evidence: minimum success rate (0.0 to 1.0)
    pub evidence_min_success_rate: f64,
}

impl Default for PipelineThresholds {
    fn default() -> Self {
        Self {
            // Pattern stage: requires N≥3 occurrences
            pattern_min_occurrences: 3,
            // Hypothesis stage: requires N≥5 with >60% success rate
            hypothesis_min_occurrences: 5,
            hypothesis_min_success_rate: 0.6,
            // Evidence stage: requires N≥10 with >80% success rate
            evidence_min_occurrences: 10,
            evidence_min_success_rate: 0.8,
        }
    }
}

/// A learning pipeline entry tracking an observation through all stages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningEntry {
    /// Unique identifier for this entry
    pub id: Uuid,
    /// The observation/event that was captured
    pub observation: Observation,
    /// Current pipeline stage
    pub stage: PipelineStage,
    /// Number of times this entry has been observed
    pub occurrence_count: u32,
    /// Number of successful outcomes
    pub success_count: u32,
    /// Number of failed outcomes
    pub failure_count: u32,
    /// Current success rate (successes / total attempts)
    pub success_rate: f64,
    /// Whether this entry has been deprecated
    pub is_deprecated: bool,
    /// Deprecation reason if deprecated
    pub deprecation_reason: Option<DeprecationReason>,
    /// When the entry was first observed
    pub created_at: DateTime<Utc>,
    /// When the entry was last updated
    pub updated_at: DateTime<Utc>,
    /// Timestamp when current stage was entered
    pub stage_entered_at: DateTime<Utc>,
    /// Stage transition history
    pub stage_history: Vec<StageTransition>,
    /// Repository scope for this entry
    pub repository: String,
    /// Additional metadata
    pub metadata: serde_json::Value,
}

impl LearningEntry {
    /// Create a new learning entry from an observation
    pub fn new(observation: Observation, repository: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            observation,
            stage: PipelineStage::Observation,
            occurrence_count: 1,
            success_count: 0,
            failure_count: 0,
            success_rate: 0.0,
            is_deprecated: false,
            deprecation_reason: None,
            created_at: now,
            updated_at: now,
            stage_entered_at: now,
            stage_history: Vec::new(),
            repository,
            metadata: serde_json::json!({}),
        }
    }

    /// Record an occurrence of this entry (observation observed again)
    pub fn record_occurrence(&mut self) {
        self.occurrence_count += 1;
        self.updated_at = Utc::now();
    }

    /// Record an outcome (success or failure)
    pub fn record_outcome(&mut self, success: bool) {
        if success {
            self.success_count += 1;
        } else {
            self.failure_count += 1;
        }
        self.updated_at = Utc::now();
        self.recalculate_success_rate();
    }

    /// Recalculate the success rate
    fn recalculate_success_rate(&mut self) {
        let total = self.success_count + self.failure_count;
        self.success_rate = if total > 0 {
            self.success_count as f64 / total as f64
        } else {
            0.0
        };
    }

    /// Check if the entry meets pattern detection threshold
    pub fn meets_pattern_threshold(&self, thresholds: &PipelineThresholds) -> bool {
        self.occurrence_count >= thresholds.pattern_min_occurrences
    }

    /// Check if the entry meets hypothesis threshold
    pub fn meets_hypothesis_threshold(&self, thresholds: &PipelineThresholds) -> bool {
        self.occurrence_count >= thresholds.hypothesis_min_occurrences
            && self.success_rate > thresholds.hypothesis_min_success_rate
    }

    /// Check if the entry meets evidence/accumulation threshold
    pub fn meets_evidence_threshold(&self, thresholds: &PipelineThresholds) -> bool {
        self.occurrence_count >= thresholds.evidence_min_occurrences
            && self.success_rate > thresholds.evidence_min_success_rate
    }

    /// Attempt to advance to the next stage
    /// Returns the new stage if transition occurred, None otherwise
    pub fn try_advance_stage(&mut self, thresholds: &PipelineThresholds) -> Option<PipelineStage> {
        let next = self.stage.next_stage()?;

        let should_advance = match &self.stage {
            PipelineStage::Observation => {
                // Advance to Pattern when we have enough occurrences
                self.meets_pattern_threshold(thresholds)
            }
            PipelineStage::Pattern => {
                // Advance to Hypothesis when we have enough occurrences AND success rate > 60%
                self.meets_hypothesis_threshold(thresholds)
            }
            PipelineStage::Hypothesis => {
                // Advance to Evidence when we have enough occurrences AND success rate > 80%
                self.meets_evidence_threshold(thresholds)
            }
            PipelineStage::Evidence => {
                // Advance to Promoted - already validated
                true
            }
            PipelineStage::Promoted | PipelineStage::Deprecated => {
                // Terminal states
                return None;
            }
        };

        if should_advance {
            let old_stage = self.stage;
            self.stage = next;
            self.stage_entered_at = Utc::now();
            self.stage_history.push(StageTransition {
                from_stage: old_stage,
                to_stage: next,
                timestamp: Utc::now(),
                occurrence_count: self.occurrence_count,
                success_rate: self.success_rate,
            });
            Some(next)
        } else {
            None
        }
    }

    /// Check if entry should be deprecated based on current thresholds
    pub fn check_deprecation(&mut self, thresholds: &PipelineThresholds) -> bool {
        // Check if entry fails to meet the minimum requirements to advance
        // and has exhausted its opportunities

        match &self.stage {
            PipelineStage::Observation => {
                // Observation fails if we have enough occurrences but pattern not met
                // This is tricky - observation can't really "fail", it just doesn't advance
                false
            }
            PipelineStage::Pattern => {
                // Pattern fails if we have enough occurrences (N≥3) but success rate is too low
                // to meet hypothesis requirements
                if self.occurrence_count >= thresholds.pattern_min_occurrences {
                    // Check if we can still form a hypothesis
                    // If occurrence_count > hypothesis_min_occurrences and success rate still < hypothesis_min_success_rate
                    if self.occurrence_count >= thresholds.hypothesis_min_occurrences
                        && self.success_rate < thresholds.hypothesis_min_success_rate
                    {
                        return true;
                    }
                }
                false
            }
            PipelineStage::Hypothesis => {
                // Hypothesis fails if we have enough occurrences but still can't meet evidence threshold
                if self.occurrence_count >= thresholds.hypothesis_min_occurrences
                    && self.success_rate < thresholds.hypothesis_min_success_rate
                {
                    return true;
                }
                // If we've exceeded evidence occurrences but still can't meet evidence threshold
                if self.occurrence_count >= thresholds.evidence_min_occurrences
                    && self.success_rate < thresholds.evidence_min_success_rate
                {
                    return true;
                }
                false
            }
            PipelineStage::Evidence => {
                // Evidence fails if we have enough occurrences but can't meet promotion threshold
                if self.occurrence_count >= thresholds.evidence_min_occurrences
                    && self.success_rate < thresholds.evidence_min_success_rate
                {
                    return true;
                }
                false
            }
            PipelineStage::Promoted | PipelineStage::Deprecated => {
                // Already at terminal state
                false
            }
        }
    }

    /// Deprecate this entry with a reason
    pub fn deprecate(&mut self, reason: DeprecationReason) {
        let old_stage = self.stage;
        self.stage = PipelineStage::Deprecated;
        self.is_deprecated = true;
        self.deprecation_reason = Some(reason);
        self.updated_at = Utc::now();
        self.stage_history.push(StageTransition {
            from_stage: old_stage,
            to_stage: PipelineStage::Deprecated,
            timestamp: Utc::now(),
            occurrence_count: self.occurrence_count,
            success_rate: self.success_rate,
        });
    }

    /// Promote this entry to knowledge
    pub fn promote(&mut self) {
        let old_stage = self.stage;
        self.stage = PipelineStage::Promoted;
        self.updated_at = Utc::now();
        self.stage_history.push(StageTransition {
            from_stage: old_stage,
            to_stage: PipelineStage::Promoted,
            timestamp: Utc::now(),
            occurrence_count: self.occurrence_count,
            success_rate: self.success_rate,
        });
    }
}

/// Reason for deprecation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeprecationReason {
    /// Failed to transition from Observation to Pattern (insufficient occurrences)
    InsufficientOccurrences,
    /// Failed to transition from Pattern to Hypothesis (success rate too low)
    LowSuccessRate,
    /// Failed to transition from Hypothesis to Evidence (success rate too low)
    HypothesisFailed,
    /// Failed to transition from Evidence to Promotion (success rate below threshold)
    EvidenceFailed,
    /// Manual deprecation by user or system
    Manual,
}

impl DeprecationReason {
    pub fn description(&self) -> &'static str {
        match self {
            DeprecationReason::InsufficientOccurrences => {
                "Insufficient occurrences to advance to next stage"
            }
            DeprecationReason::LowSuccessRate => {
                "Success rate too low to meet threshold requirements"
            }
            DeprecationReason::HypothesisFailed => "Failed to meet Hypothesis stage requirements",
            DeprecationReason::EvidenceFailed => {
                "Failed to meet Evidence stage requirements for promotion"
            }
            DeprecationReason::Manual => "Manually deprecated",
        }
    }
}

/// A recorded stage transition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageTransition {
    pub from_stage: PipelineStage,
    pub to_stage: PipelineStage,
    pub timestamp: DateTime<Utc>,
    pub occurrence_count: u32,
    pub success_rate: f64,
}

/// The observation/event that triggered this learning entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    /// Type of observation
    pub observation_type: ObservationType,
    /// Description of what was observed
    pub description: String,
    /// Context/environment where observation occurred
    pub context: String,
    /// Source task ID if applicable
    pub source_task_id: Option<Uuid>,
    /// Timestamp when observation occurred
    pub observed_at: DateTime<Utc>,
    /// Additional metadata
    pub metadata: serde_json::Value,
}

impl Observation {
    /// Create a new observation
    pub fn new(observation_type: ObservationType, description: String, context: String) -> Self {
        Self {
            observation_type,
            description,
            context,
            source_task_id: None,
            observed_at: Utc::now(),
            metadata: serde_json::json!({}),
        }
    }

    /// Create with a source task ID
    pub fn with_source_task(mut self, task_id: Uuid) -> Self {
        self.source_task_id = Some(task_id);
        self
    }
}

/// Types of observations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationType {
    /// Tool sequence observed
    ToolSequence,
    /// Pattern detected in code
    CodePattern,
    /// Error pattern observed
    ErrorPattern,
    /// Success pattern observed
    SuccessPattern,
    /// Convention learned
    Convention,
    /// Anti-pattern detected
    AntiPattern,
}

impl ObservationType {
    pub fn name(&self) -> &'static str {
        match self {
            ObservationType::ToolSequence => "Tool Sequence",
            ObservationType::CodePattern => "Code Pattern",
            ObservationType::ErrorPattern => "Error Pattern",
            ObservationType::SuccessPattern => "Success Pattern",
            ObservationType::Convention => "Convention",
            ObservationType::AntiPattern => "Anti-Pattern",
        }
    }
}

/// Result of processing an observation through the pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineResult {
    pub entry_id: Uuid,
    pub previous_stage: PipelineStage,
    pub current_stage: PipelineStage,
    pub advancement_occurred: bool,
    pub deprecation_occurred: bool,
    pub deprecation_reason: Option<DeprecationReason>,
    pub promotion_occurred: bool,
    pub occurrence_count: u32,
    pub success_rate: f64,
}

/// The 5-stage learning pipeline processor
#[derive(Debug, Clone)]
pub struct LearningPipeline {
    thresholds: PipelineThresholds,
    entries: HashMap<Uuid, LearningEntry>,
}

impl LearningPipeline {
    /// Create a new learning pipeline with default thresholds
    pub fn new() -> Self {
        Self {
            thresholds: PipelineThresholds::default(),
            entries: HashMap::new(),
        }
    }

    /// Create with custom thresholds
    pub fn with_thresholds(thresholds: PipelineThresholds) -> Self {
        Self {
            thresholds,
            entries: HashMap::new(),
        }
    }

    /// Get a threshold reference
    pub fn thresholds(&self) -> &PipelineThresholds {
        &self.thresholds
    }

    /// Get an entry by ID
    pub fn get_entry(&self, id: Uuid) -> Option<&LearningEntry> {
        self.entries.get(&id)
    }

    /// Get a mutable entry by ID
    pub fn get_entry_mut(&mut self, id: Uuid) -> Option<&mut LearningEntry> {
        self.entries.get_mut(&id)
    }

    /// Get all entries at a specific stage
    pub fn get_entries_at_stage(&self, stage: PipelineStage) -> Vec<&LearningEntry> {
        self.entries.values().filter(|e| e.stage == stage).collect()
    }

    /// Get all non-deprecated entries
    pub fn get_active_entries(&self) -> Vec<&LearningEntry> {
        self.entries.values().filter(|e| !e.is_deprecated).collect()
    }

    /// Process a new observation, creating a new entry or updating existing
    pub fn observe(&mut self, observation: Observation, repository: String) -> PipelineResult {
        // Check if we already have an entry for this observation
        // Use observation content hash to match
        let observation_key = self.observation_key(&observation);

        // Find existing entry by observation content
        let existing_id = self
            .entries
            .values()
            .find(|e| {
                self.observation_key(&e.observation) == observation_key
                    && e.repository == repository
            })
            .map(|e| e.id);

        match existing_id {
            Some(id) => {
                // Update existing entry
                self.update_entry(id, true)
            }
            None => {
                // Create new entry
                self.create_entry(observation, repository)
            }
        }
    }

    /// Generate a key for an observation to match entries
    fn observation_key(&self, observation: &Observation) -> String {
        format!(
            "{}:{}:{}",
            observation.observation_type.name(),
            observation.description,
            observation.context
        )
    }

    /// Create a new entry from an observation
    fn create_entry(&mut self, observation: Observation, repository: String) -> PipelineResult {
        let mut entry = LearningEntry::new(observation, repository);
        let entry_id = entry.id;

        // Try to advance from Observation to Pattern
        let previous_stage = entry.stage;
        let advancement = entry.try_advance_stage(&self.thresholds);
        let advancement_occurred = advancement.is_some();

        let current_stage = entry.stage;

        let result = PipelineResult {
            entry_id,
            previous_stage,
            current_stage,
            advancement_occurred,
            deprecation_occurred: false,
            deprecation_reason: None,
            promotion_occurred: false,
            occurrence_count: entry.occurrence_count,
            success_rate: entry.success_rate,
        };

        self.entries.insert(entry_id, entry);
        result
    }

    /// Update an existing entry with a new occurrence
    fn update_entry(&mut self, entry_id: Uuid, success: bool) -> PipelineResult {
        let entry = self.entries.get_mut(&entry_id).expect("Entry must exist");

        let previous_stage = entry.stage;

        // Record occurrence and outcome
        entry.record_occurrence();
        entry.record_outcome(success);

        // Try to advance stages
        let advancement = entry.try_advance_stage(&self.thresholds);

        // Check for deprecation
        let deprecation_occurred = entry.check_deprecation(&self.thresholds);
        let mut deprecation_reason = None;
        if deprecation_occurred && !entry.is_deprecated {
            // Determine the reason based on current stage
            deprecation_reason = Some(match &entry.stage {
                PipelineStage::Observation => DeprecationReason::InsufficientOccurrences,
                PipelineStage::Pattern => DeprecationReason::LowSuccessRate,
                PipelineStage::Hypothesis => DeprecationReason::HypothesisFailed,
                PipelineStage::Evidence => DeprecationReason::EvidenceFailed,
                _ => DeprecationReason::Manual,
            });
            entry.deprecate(*deprecation_reason.as_ref().unwrap());
        }

        // Check for promotion - only from Evidence stage when thresholds are met
        let promotion_occurred = if previous_stage == PipelineStage::Evidence
            && entry.stage == PipelineStage::Evidence
            && entry.meets_evidence_threshold(&self.thresholds)
        {
            entry.promote();
            true
        } else {
            false
        };

        PipelineResult {
            entry_id,
            previous_stage,
            current_stage: entry.stage,
            advancement_occurred: advancement.is_some(),
            deprecation_occurred,
            deprecation_reason,
            promotion_occurred,
            occurrence_count: entry.occurrence_count,
            success_rate: entry.success_rate,
        }
    }

    /// Record a success outcome for an existing entry
    pub fn record_success(&mut self, entry_id: Uuid) -> Option<PipelineResult> {
        if !self.entries.contains_key(&entry_id) {
            return None;
        }
        Some(self.update_entry(entry_id, true))
    }

    /// Record a failure outcome for an existing entry
    pub fn record_failure(&mut self, entry_id: Uuid) -> Option<PipelineResult> {
        if !self.entries.contains_key(&entry_id) {
            return None;
        }
        Some(self.update_entry(entry_id, false))
    }

    /// Manually deprecate an entry
    pub fn deprecate_entry(&mut self, entry_id: Uuid, reason: DeprecationReason) -> bool {
        if let Some(entry) = self.entries.get_mut(&entry_id) {
            entry.deprecate(reason);
            true
        } else {
            false
        }
    }

    /// Manually promote an entry
    pub fn promote_entry(&mut self, entry_id: Uuid) -> bool {
        if let Some(entry) = self.entries.get_mut(&entry_id) {
            entry.promote();
            true
        } else {
            false
        }
    }

    /// Get statistics about the pipeline
    pub fn stats(&self) -> PipelineStats {
        let mut stage_counts: HashMap<PipelineStage, usize> = HashMap::new();
        let mut total_occurrences = 0u64;
        let mut total_successes = 0u64;

        for entry in self.entries.values() {
            *stage_counts.entry(entry.stage).or_insert(0) += 1;
            total_occurrences += entry.occurrence_count as u64;
            total_successes += entry.success_count as u64;
        }

        PipelineStats {
            total_entries: self.entries.len(),
            stage_counts,
            total_occurrences,
            total_successes,
            overall_success_rate: if total_occurrences > 0 {
                total_successes as f64 / total_occurrences as f64
            } else {
                0.0
            },
        }
    }
}

impl Default for LearningPipeline {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about the pipeline state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStats {
    pub total_entries: usize,
    pub stage_counts: HashMap<PipelineStage, usize>,
    pub total_occurrences: u64,
    pub total_successes: u64,
    pub overall_success_rate: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_observation() -> Observation {
        Observation::new(
            ObservationType::ToolSequence,
            "read_file -> edit_file -> shell".to_string(),
            "file editing workflow".to_string(),
        )
    }

    #[test]
    fn test_pipeline_stage_names() {
        assert_eq!(PipelineStage::Observation.name(), "Observation");
        assert_eq!(PipelineStage::Pattern.name(), "Pattern Detection");
        assert_eq!(PipelineStage::Hypothesis.name(), "Hypothesis");
        assert_eq!(PipelineStage::Evidence.name(), "Evidence Accumulation");
        assert_eq!(PipelineStage::Promoted.name(), "Promoted");
        assert_eq!(PipelineStage::Deprecated.name(), "Deprecated");
    }

    #[test]
    fn test_pipeline_stage_next() {
        assert_eq!(
            PipelineStage::Observation.next_stage(),
            Some(PipelineStage::Pattern)
        );
        assert_eq!(
            PipelineStage::Pattern.next_stage(),
            Some(PipelineStage::Hypothesis)
        );
        assert_eq!(
            PipelineStage::Hypothesis.next_stage(),
            Some(PipelineStage::Evidence)
        );
        assert_eq!(
            PipelineStage::Evidence.next_stage(),
            Some(PipelineStage::Promoted)
        );
        assert_eq!(PipelineStage::Promoted.next_stage(), None);
        assert_eq!(PipelineStage::Deprecated.next_stage(), None);
    }

    #[test]
    fn test_pipeline_stage_is_terminal() {
        assert!(!PipelineStage::Observation.is_terminal());
        assert!(!PipelineStage::Pattern.is_terminal());
        assert!(!PipelineStage::Hypothesis.is_terminal());
        assert!(!PipelineStage::Evidence.is_terminal());
        assert!(PipelineStage::Promoted.is_terminal());
        assert!(PipelineStage::Deprecated.is_terminal());
    }

    #[test]
    fn test_default_thresholds() {
        let thresholds = PipelineThresholds::default();
        assert_eq!(thresholds.pattern_min_occurrences, 3);
        assert_eq!(thresholds.hypothesis_min_occurrences, 5);
        assert_eq!(thresholds.hypothesis_min_success_rate, 0.6);
        assert_eq!(thresholds.evidence_min_occurrences, 10);
        assert_eq!(thresholds.evidence_min_success_rate, 0.8);
    }

    #[test]
    fn test_learning_entry_new() {
        let observation = create_observation();
        let entry = LearningEntry::new(observation, "test-repo".to_string());

        assert_eq!(entry.stage, PipelineStage::Observation);
        assert_eq!(entry.occurrence_count, 1);
        assert_eq!(entry.success_count, 0);
        assert_eq!(entry.failure_count, 0);
        assert_eq!(entry.success_rate, 0.0);
        assert!(!entry.is_deprecated);
        assert!(entry.stage_history.is_empty());
    }

    #[test]
    fn test_learning_entry_record_occurrence() {
        let observation = create_observation();
        let mut entry = LearningEntry::new(observation, "test-repo".to_string());

        entry.record_occurrence();
        assert_eq!(entry.occurrence_count, 2);

        entry.record_occurrence();
        assert_eq!(entry.occurrence_count, 3);
    }

    #[test]
    fn test_learning_entry_record_outcome() {
        let observation = create_observation();
        let mut entry = LearningEntry::new(observation, "test-repo".to_string());

        entry.record_outcome(true);
        assert_eq!(entry.success_count, 1);
        assert_eq!(entry.failure_count, 0);
        assert_eq!(entry.success_rate, 1.0);

        entry.record_outcome(false);
        assert_eq!(entry.success_count, 1);
        assert_eq!(entry.failure_count, 1);
        assert_eq!(entry.success_rate, 0.5);
    }

    #[test]
    fn test_learning_entry_meets_thresholds() {
        let observation = create_observation();
        let mut entry = LearningEntry::new(observation, "test-repo".to_string());
        let thresholds = PipelineThresholds::default();

        // Initially doesn't meet any thresholds
        assert!(!entry.meets_pattern_threshold(&thresholds));
        assert!(!entry.meets_hypothesis_threshold(&thresholds));
        assert!(!entry.meets_evidence_threshold(&thresholds));

        // Add occurrences to meet pattern threshold (3)
        entry.occurrence_count = 3;
        assert!(entry.meets_pattern_threshold(&thresholds));
        assert!(!entry.meets_hypothesis_threshold(&thresholds));
        assert!(!entry.meets_evidence_threshold(&thresholds));

        // Add occurrences to meet hypothesis threshold (5) but no successes
        entry.occurrence_count = 5;
        assert!(entry.meets_pattern_threshold(&thresholds));
        assert!(!entry.meets_hypothesis_threshold(&thresholds)); // Need success rate > 60%

        // Add successes to meet hypothesis threshold
        entry.success_count = 4;
        entry.failure_count = 1;
        entry.recalculate_success_rate(); // 4/5 = 0.8 > 0.6
        assert!(entry.meets_hypothesis_threshold(&thresholds));

        // Add more to meet evidence threshold (10 with >80% success)
        entry.occurrence_count = 10;
        entry.success_count = 9;
        entry.failure_count = 1;
        entry.recalculate_success_rate(); // 9/10 = 0.9 > 0.8
        assert!(entry.meets_evidence_threshold(&thresholds));
    }

    #[test]
    fn test_observation_creation() {
        let obs = Observation::new(
            ObservationType::CodePattern,
            "use std::collections::HashMap".to_string(),
            "Rust imports".to_string(),
        );

        assert_eq!(obs.observation_type, ObservationType::CodePattern);
        assert_eq!(obs.description, "use std::collections::HashMap");
        assert!(obs.source_task_id.is_none());

        let obs_with_task = obs.with_source_task(Uuid::new_v4());
        assert!(obs_with_task.source_task_id.is_some());
    }

    #[test]
    fn test_learning_pipeline_new_entry() {
        let mut pipeline = LearningPipeline::new();
        let observation = create_observation();

        let result = pipeline.observe(observation, "test-repo".to_string());

        // New entry starts at Observation, doesn't advance on first occurrence
        assert_eq!(result.previous_stage, PipelineStage::Observation);
        assert_eq!(result.current_stage, PipelineStage::Observation);
        assert!(!result.advancement_occurred);
        assert!(!result.deprecation_occurred);
        assert!(!result.promotion_occurred);
        assert_eq!(result.occurrence_count, 1);
    }

    #[test]
    fn test_learning_pipeline_advances_through_stages() {
        let mut pipeline = LearningPipeline::new();
        let observation = create_observation();

        // First observation
        let result1 = pipeline.observe(observation.clone(), "test-repo".to_string());
        assert_eq!(result1.current_stage, PipelineStage::Observation);
        let entry_id = result1.entry_id;

        // Add occurrences until we reach Pattern stage (3 occurrences)
        // Observation -> Pattern requires N >= 3 occurrences
        for _ in 0..2 {
            pipeline.record_success(entry_id);
        }

        let entry = pipeline.get_entry(entry_id).unwrap();
        assert_eq!(entry.stage, PipelineStage::Pattern);
        assert_eq!(entry.occurrence_count, 3);
    }

    #[test]
    fn test_learning_pipeline_hypothesis_requires_success_rate() {
        let mut pipeline = LearningPipeline::new();
        let observation = create_observation();

        let result = pipeline.observe(observation.clone(), "test-repo".to_string());
        let entry_id = result.entry_id;

        // Add 4 more occurrences with all successes to reach Hypothesis
        // Hypothesis requires N >= 5 with success rate > 60%
        for _ in 0..4 {
            pipeline.record_success(entry_id);
        }

        let entry = pipeline.get_entry(entry_id).unwrap();
        // Should be at Hypothesis stage: 5 occurrences, 100% success rate (>60%)
        assert_eq!(entry.stage, PipelineStage::Hypothesis);
    }

    #[test]
    fn test_learning_pipeline_hypothesis_fails_with_low_success_rate() {
        let mut pipeline = LearningPipeline::new();
        let observation = create_observation();

        let result = pipeline.observe(observation.clone(), "test-repo".to_string());
        let entry_id = result.entry_id;

        // Add more occurrences but with low success rate (3 fails, 2 successes)
        // Pattern -> Hypothesis: N >= 5 with success rate > 60%
        // 2 successes / 5 total = 40% < 60%, so should NOT advance to Hypothesis
        for _ in 0..2 {
            pipeline.record_success(entry_id);
        }
        for _ in 0..3 {
            pipeline.record_failure(entry_id);
        }

        let entry = pipeline.get_entry(entry_id).unwrap();
        // Should be deprecated at Pattern stage with LowSuccessRate
        // (5 occurrences, 40% success rate - can't advance to Hypothesis)
        assert!(entry.is_deprecated);
        assert!(matches!(
            entry.deprecation_reason,
            Some(DeprecationReason::LowSuccessRate)
        ));
    }

    #[test]
    fn test_learning_pipeline_full_promotion_path() {
        let mut pipeline = LearningPipeline::new();
        let observation = create_observation();

        let result = pipeline.observe(observation.clone(), "test-repo".to_string());
        let entry_id = result.entry_id;

        // Progress to Evidence stage first
        // Pattern (N>=3) -> Hypothesis (N>=5, >60% success) -> Evidence (N>=10, >80% success)

        // Add 9 more successes to get 10 total (Evidence requires 10 occurrences with >80% success)
        for _ in 0..9 {
            pipeline.record_success(entry_id);
        }

        let entry = pipeline.get_entry(entry_id).unwrap();
        // 10 total occurrences, 100% success rate, should advance to Evidence
        assert_eq!(entry.stage, PipelineStage::Evidence);

        // Another success should promote to Promoted
        pipeline.record_success(entry_id);

        let entry = pipeline.get_entry(entry_id).unwrap();
        assert_eq!(entry.stage, PipelineStage::Promoted);
        assert!(entry
            .stage_history
            .iter()
            .any(|t| t.to_stage == PipelineStage::Promoted));
    }

    #[test]
    fn test_learning_pipeline_deprecation_at_hypothesis() {
        let mut pipeline = LearningPipeline::new();
        let observation = create_observation();

        let result = pipeline.observe(observation.clone(), "test-repo".to_string());
        let entry_id = result.entry_id;

        // Add occurrences with low success rate to trigger deprecation
        // Pattern requires N>=3 (met at 3 occurrences)
        // Hypothesis requires N>=5 with >60% success (not met at 5 occurrences with low success)
        // With 3 successes and 3 failures = 50% success rate, should deprecate at Pattern stage
        for _ in 0..3 {
            pipeline.record_failure(entry_id);
        }
        for _ in 0..3 {
            pipeline.record_success(entry_id);
        }
        // Now at 6 occurrences with 3 successes = 50% success rate
        // Still below 60% hypothesis threshold, check_deprecation should trigger

        let entry = pipeline.get_entry(entry_id).unwrap();
        // Entry should be deprecated with LowSuccessRate (failed to advance from Pattern to Hypothesis)
        assert!(entry.is_deprecated);
        assert!(matches!(
            entry.deprecation_reason,
            Some(DeprecationReason::LowSuccessRate)
        ));
    }

    #[test]
    fn test_learning_pipeline_deprecation_at_evidence() {
        let mut pipeline = LearningPipeline::new();
        let observation = create_observation();

        let result = pipeline.observe(observation.clone(), "test-repo".to_string());
        let entry_id = result.entry_id;

        // Get to Hypothesis stage first (N>=5 with >60% success)
        for _ in 0..4 {
            pipeline.record_success(entry_id);
        }
        // Now at 5 occurrences, 100% success - should be at Hypothesis

        let entry = pipeline.get_entry(entry_id).unwrap();
        assert_eq!(entry.stage, PipelineStage::Hypothesis);

        // Continue with failures to prevent Evidence promotion
        // Add 5 more with low success rate (3 fails, 2 successes)
        // Total: 10 occurrences, 5 successes = 50% success rate
        // Evidence requires >80% success, so should deprecate
        for _ in 0..2 {
            pipeline.record_success(entry_id);
        }
        for _ in 0..3 {
            pipeline.record_failure(entry_id);
        }

        let entry = pipeline.get_entry(entry_id).unwrap();
        // Should be deprecated - the Hypothesis stage failed to meet Evidence requirements
        assert!(entry.is_deprecated);
        assert!(matches!(
            entry.deprecation_reason,
            Some(DeprecationReason::HypothesisFailed)
        ));
    }

    #[test]
    fn test_learning_pipeline_stats() {
        let mut pipeline = LearningPipeline::new();
        let observation = create_observation();

        // Create multiple entries
        let result1 = pipeline.observe(observation.clone(), "test-repo".to_string());
        let result2 = pipeline.observe(
            Observation::new(
                ObservationType::CodePattern,
                "use chrono::Utc".to_string(),
                "Rust imports".to_string(),
            ),
            "test-repo".to_string(),
        );

        let entry1_id = result1.entry_id;
        let entry2_id = result2.entry_id;

        // Progress entry1 to Pattern stage
        pipeline.record_success(entry1_id);
        pipeline.record_success(entry1_id);

        // Progress entry2 to Hypothesis
        for _ in 0..4 {
            pipeline.record_success(entry2_id);
        }

        let stats = pipeline.stats();

        assert_eq!(stats.total_entries, 2);
        assert_eq!(
            *stats
                .stage_counts
                .get(&PipelineStage::Pattern)
                .unwrap_or(&0),
            1
        );
        assert_eq!(
            *stats
                .stage_counts
                .get(&PipelineStage::Hypothesis)
                .unwrap_or(&0),
            1
        );
        assert_eq!(
            *stats
                .stage_counts
                .get(&PipelineStage::Observation)
                .unwrap_or(&0),
            0
        );
    }

    #[test]
    fn test_learning_pipeline_get_entries_at_stage() {
        let mut pipeline = LearningPipeline::new();
        let observation = create_observation();

        let result1 = pipeline.observe(observation.clone(), "test-repo".to_string());
        let result2 = pipeline.observe(
            Observation::new(
                ObservationType::CodePattern,
                "use chrono::Utc".to_string(),
                "Rust imports".to_string(),
            ),
            "test-repo".to_string(),
        );

        let entry1_id = result1.entry_id;
        let entry2_id = result2.entry_id;

        // Progress entry1 to Pattern stage
        pipeline.record_success(entry1_id);
        pipeline.record_success(entry1_id);

        // Progress entry2 to Hypothesis stage (5 occurrences with 100% success rate)
        for _ in 0..4 {
            pipeline.record_success(entry2_id);
        }

        let pattern_entries = pipeline.get_entries_at_stage(PipelineStage::Pattern);
        assert_eq!(pattern_entries.len(), 1);
        assert_eq!(pattern_entries[0].id, entry1_id);

        let hypothesis_entries = pipeline.get_entries_at_stage(PipelineStage::Hypothesis);
        assert_eq!(hypothesis_entries.len(), 1);
        assert_eq!(hypothesis_entries[0].id, entry2_id);
    }

    #[test]
    fn test_stage_transition_history() {
        let mut pipeline = LearningPipeline::new();
        let observation = create_observation();

        let result = pipeline.observe(observation, "test-repo".to_string());
        let entry_id = result.entry_id;

        // Progress through stages
        for _ in 0..2 {
            pipeline.record_success(entry_id);
        }
        // Should be at Pattern now (3 occurrences)

        for _ in 0..2 {
            pipeline.record_success(entry_id);
        }
        // Should be at Hypothesis now (5 occurrences, 100% success)

        let entry = pipeline.get_entry(entry_id).unwrap();
        assert_eq!(entry.stage_history.len(), 2); // Observation->Pattern, Pattern->Hypothesis

        // Check stage names
        assert_eq!(
            entry.stage_history[0].from_stage,
            PipelineStage::Observation
        );
        assert_eq!(entry.stage_history[0].to_stage, PipelineStage::Pattern);
        assert_eq!(entry.stage_history[1].from_stage, PipelineStage::Pattern);
        assert_eq!(entry.stage_history[1].to_stage, PipelineStage::Hypothesis);
    }

    #[test]
    fn test_manual_deprecation() {
        let mut pipeline = LearningPipeline::new();
        let observation = create_observation();

        let result = pipeline.observe(observation, "test-repo".to_string());
        let entry_id = result.entry_id;

        // Manually deprecate
        let success = pipeline.deprecate_entry(entry_id, DeprecationReason::Manual);
        assert!(success);

        let entry = pipeline.get_entry(entry_id).unwrap();
        assert!(entry.is_deprecated);
        assert!(matches!(
            entry.deprecation_reason,
            Some(DeprecationReason::Manual)
        ));
        assert_eq!(entry.stage, PipelineStage::Deprecated);
    }

    #[test]
    fn test_manual_promotion() {
        let mut pipeline = LearningPipeline::new();
        let observation = create_observation();

        let result = pipeline.observe(observation, "test-repo".to_string());
        let entry_id = result.entry_id;

        // Progress to Evidence
        for _ in 0..9 {
            pipeline.record_success(entry_id);
        }

        {
            let entry = pipeline.get_entry(entry_id).unwrap();
            assert_eq!(entry.stage, PipelineStage::Evidence);
        }

        // Manually promote
        let success = pipeline.promote_entry(entry_id);
        assert!(success);

        let entry = pipeline.get_entry(entry_id).unwrap();
        assert_eq!(entry.stage, PipelineStage::Promoted);
    }

    #[test]
    fn test_observation_type_names() {
        assert_eq!(ObservationType::ToolSequence.name(), "Tool Sequence");
        assert_eq!(ObservationType::CodePattern.name(), "Code Pattern");
        assert_eq!(ObservationType::ErrorPattern.name(), "Error Pattern");
        assert_eq!(ObservationType::SuccessPattern.name(), "Success Pattern");
        assert_eq!(ObservationType::Convention.name(), "Convention");
        assert_eq!(ObservationType::AntiPattern.name(), "Anti-Pattern");
    }

    #[test]
    fn test_deprecation_reason_description() {
        assert!(DeprecationReason::InsufficientOccurrences
            .description()
            .contains("Insufficient"));
        assert!(DeprecationReason::LowSuccessRate
            .description()
            .contains("Success rate"));
        assert!(DeprecationReason::HypothesisFailed
            .description()
            .contains("Hypothesis"));
        assert!(DeprecationReason::EvidenceFailed
            .description()
            .contains("Evidence"));
        assert!(DeprecationReason::Manual.description().contains("Manually"));
    }

    #[test]
    fn test_pipeline_with_custom_thresholds() {
        let custom_thresholds = PipelineThresholds {
            pattern_min_occurrences: 2, // Lower threshold for testing
            hypothesis_min_occurrences: 3,
            hypothesis_min_success_rate: 0.5, // 50% instead of 60%
            evidence_min_occurrences: 5,
            evidence_min_success_rate: 0.7, // 70% instead of 80%
        };

        let mut pipeline = LearningPipeline::with_thresholds(custom_thresholds);
        let observation = create_observation();

        let result = pipeline.observe(observation, "test-repo".to_string());
        let entry_id = result.entry_id;

        // With custom thresholds, should advance to Pattern with just 2 occurrences
        pipeline.record_success(entry_id);

        let entry = pipeline.get_entry(entry_id).unwrap();
        assert_eq!(entry.stage, PipelineStage::Pattern);

        // And Hypothesis with 3 occurrences and 50% success
        pipeline.record_failure(entry_id);
        pipeline.record_success(entry_id);

        let entry = pipeline.get_entry(entry_id).unwrap();
        assert_eq!(entry.stage, PipelineStage::Hypothesis);
    }
}
