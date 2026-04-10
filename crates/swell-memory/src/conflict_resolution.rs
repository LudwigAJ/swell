// conflict_resolution.rs - Memory conflict detection and resolution
//
// This module provides functionality to detect contradictory memories and resolve
// them using configurable resolution rules.
//
// Conflict detection is based on semantic similarity between memories. When a new
// memory is stored that conflicts with existing memories in the same scope, the
// conflict resolution system determines which memory to keep based on:
//
// - Resolution strategies: newer_wins, higher_confidence_wins, operator_overrides
// - Provenance tracking: superseded memories retain links for audit trail
// - Logging: all conflict resolution decisions are logged for transparency
//
// Architecture:
// - MemoryConflictDetector: identifies potential conflicts between memories
// - ConflictResolver: applies resolution rules to determine winner
// - ConflictResolutionLog: maintains audit trail of all resolution decisions

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Maximum cosine distance to consider memories as potentially conflicting.
/// Memories with distance between 0.15 and 0.40 are flagged as potential conflicts.
/// Below 0.15 = too similar (deduplication)
/// 0.15-0.40 = potential conflict (needs resolution)
/// Above 0.40 = sufficiently different (no conflict)
pub const CONFLICT_DISTANCE_MIN: f32 = 0.15;
pub const CONFLICT_DISTANCE_MAX: f32 = 0.40;

/// Confidence threshold below which memories are considered low-confidence
/// and more likely to be superseded in conflicts.
pub const LOW_CONFIDENCE_THRESHOLD: f32 = 0.5;

/// Resolution strategies for handling memory conflicts
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionStrategy {
    /// Newer memory wins over older memory
    NewerWins,
    /// Higher confidence memory wins over lower confidence
    #[default]
    HigherConfidenceWins,
    /// Operator-provided memories override agent-generated memories
    OperatorOverrides,
    /// Keep the existing (original) memory and reject the new one
    KeepOriginal,
    /// Keep the new memory and mark the existing one as superseded
    KeepNew,
    /// Merge content from both memories (concatenation)
    Merge,
    /// No automatic resolution - requires manual intervention
    Manual,
}

impl fmt::Display for ResolutionStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolutionStrategy::NewerWins => write!(f, "newer_wins"),
            ResolutionStrategy::HigherConfidenceWins => write!(f, "higher_confidence_wins"),
            ResolutionStrategy::OperatorOverrides => write!(f, "operator_overrides"),
            ResolutionStrategy::KeepOriginal => write!(f, "keep_original"),
            ResolutionStrategy::KeepNew => write!(f, "keep_new"),
            ResolutionStrategy::Merge => write!(f, "merge"),
            ResolutionStrategy::Manual => write!(f, "manual"),
        }
    }
}

/// Configuration for conflict resolution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictResolutionConfig {
    /// Primary resolution strategy when conflicts are detected
    pub primary_strategy: ResolutionStrategy,
    /// Fallback strategy if primary results in tie or manual resolution
    pub fallback_strategy: ResolutionStrategy,
    /// Minimum confidence difference to consider one memory definitively stronger
    pub confidence_difference_threshold: f32,
    /// Enable operator override (operator memories always win)
    pub enable_operator_override: bool,
    /// Enable conflict logging for audit trail
    pub enable_logging: bool,
    /// Maximum age difference (in days) to consider memories as "same generation"
    pub same_generation_days: i64,
}

impl Default for ConflictResolutionConfig {
    fn default() -> Self {
        Self {
            primary_strategy: ResolutionStrategy::HigherConfidenceWins,
            fallback_strategy: ResolutionStrategy::NewerWins,
            confidence_difference_threshold: 0.2,
            enable_operator_override: true,
            enable_logging: true,
            same_generation_days: 7,
        }
    }
}

/// Represents a detected conflict between two or more memories
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConflict {
    /// Unique identifier for this conflict
    pub id: Uuid,
    /// IDs of memories involved in this conflict
    pub memory_ids: Vec<Uuid>,
    /// The conflicting memories (id -> memory summary)
    pub conflicting_memories: Vec<MemoryConflictSummary>,
    /// Semantic distance between the conflicting memories
    pub distance: f32,
    /// Type of conflict detected
    pub conflict_type: ConflictType,
    /// Timestamp when conflict was detected
    pub detected_at: DateTime<Utc>,
    /// Repository scope of the conflict
    pub repository: String,
}

/// Types of conflicts that can occur between memories
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictType {
    /// Direct contradiction: one says X, another says not-X
    Contradiction,
    /// Outdated information: newer data supersedes older
    OutdatedSupersession,
    /// Partial overlap: memories contain some conflicting information
    PartialOverlap,
    /// Confidence conflict: different confidence levels for same fact
    ConfidenceConflict,
    /// Provenance conflict: conflicting sources claim different facts
    ProvenanceConflict,
}

impl fmt::Display for ConflictType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConflictType::Contradiction => write!(f, "contradiction"),
            ConflictType::OutdatedSupersession => write!(f, "outdated_supersession"),
            ConflictType::PartialOverlap => write!(f, "partial_overlap"),
            ConflictType::ConfidenceConflict => write!(f, "confidence_conflict"),
            ConflictType::ProvenanceConflict => write!(f, "provenance_conflict"),
        }
    }
}

/// Summary of a memory involved in a conflict
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConflictSummary {
    pub id: Uuid,
    pub label: String,
    pub content_preview: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub confidence: f32,
    pub source: MemorySource,
    pub is_operator_feedback: bool,
}

/// Source of a memory - determines priority in conflict resolution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    /// Memory created from successful task execution
    TaskSuccess,
    /// Memory created from failed/rejected task
    TaskFailure,
    /// Memory extracted from operator feedback (CLAUDE.md, AGENTS.md)
    OperatorFeedback,
    /// Memory created from pattern learning
    PatternLearning,
    /// Memory created from skill extraction
    SkillExtraction,
    /// Memory manually created by user
    Manual,
    /// Unknown or legacy source
    #[default]
    Unknown,
}

impl MemorySource {
    /// Returns true if this source has higher priority than another
    /// Operator feedback always has highest priority
    pub fn outranks(&self, other: &MemorySource) -> bool {
        // Same source cannot outrank itself
        if *self == *other {
            return false;
        }

        // Operator feedback always wins over non-operator
        if *self == MemorySource::OperatorFeedback && *other != MemorySource::OperatorFeedback {
            return true;
        }
        if *other == MemorySource::OperatorFeedback {
            return false;
        }

        // Task success outranks task failure
        match (self, other) {
            (MemorySource::TaskSuccess, MemorySource::TaskFailure) => true,
            (MemorySource::TaskFailure, MemorySource::TaskSuccess) => false,
            // Manual entries outrank learned ones
            (MemorySource::Manual, MemorySource::PatternLearning) => true,
            (MemorySource::PatternLearning, MemorySource::Manual) => false,
            (MemorySource::Manual, MemorySource::SkillExtraction) => true,
            (MemorySource::SkillExtraction, MemorySource::Manual) => false,
            // Same tier = no outrank
            _ => false,
        }
    }
}

/// Result of resolving a memory conflict
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictResolutionResult {
    /// The conflict that was resolved
    pub conflict: MemoryConflict,
    /// The winning memory ID
    pub winner_id: Uuid,
    /// The losing memory ID(s)
    pub loser_ids: Vec<Uuid>,
    /// The resolution strategy used
    pub strategy_used: ResolutionStrategy,
    /// Reason for the resolution decision
    pub resolution_reason: String,
    /// Whether the winning memory was updated or stays as-is
    pub winner_updated: bool,
    /// IDs of memories marked as superseded
    pub superseded_ids: Vec<Uuid>,
    /// Timestamp of resolution
    pub resolved_at: DateTime<Utc>,
}

/// A log entry for conflict resolution audit trail
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictResolutionLogEntry {
    pub id: Uuid,
    pub conflict_id: Uuid,
    pub memory_ids: Vec<Uuid>,
    pub winner_id: Uuid,
    pub loser_ids: Vec<Uuid>,
    pub strategy: ResolutionStrategy,
    pub conflict_type: ConflictType,
    pub distance: f32,
    pub resolution_reason: String,
    pub repository: String,
    pub resolved_at: DateTime<Utc>,
}

impl ConflictResolutionLogEntry {
    /// Create a new log entry from a resolution result
    pub fn from_result(result: &ConflictResolutionResult, repository: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            conflict_id: result.conflict.id,
            memory_ids: result.conflict.memory_ids.clone(),
            winner_id: result.winner_id,
            loser_ids: result.loser_ids.clone(),
            strategy: result.strategy_used,
            conflict_type: result.conflict.conflict_type,
            distance: result.conflict.distance,
            resolution_reason: result.resolution_reason.clone(),
            repository,
            resolved_at: result.resolved_at,
        }
    }
}

/// Memory conflict detector - identifies potential conflicts between memories
pub struct MemoryConflictDetector {
    config: ConflictResolutionConfig,
}

impl MemoryConflictDetector {
    /// Create a new conflict detector with default configuration
    pub fn new() -> Self {
        Self {
            config: ConflictResolutionConfig::default(),
        }
    }

    /// Create a new conflict detector with custom configuration
    pub fn with_config(config: ConflictResolutionConfig) -> Self {
        Self { config }
    }

    /// Check if two memories are potentially conflicting
    /// Returns Some(MemoryConflict) if conflict detected, None otherwise
    pub fn detect_conflict(
        &self,
        memory1: &ConflictMemoryInfo,
        memory2: &ConflictMemoryInfo,
    ) -> Option<MemoryConflict> {
        // Must be same repository to conflict
        if memory1.repository != memory2.repository {
            return None;
        }

        // Must be in same scope (label/context) to conflict
        if memory1.label != memory2.label {
            return None;
        }

        // Calculate semantic distance using embeddings if available
        let distance = if let (Some(emb1), Some(emb2)) =
            (&memory1.embedding, &memory2.embedding)
        {
            Self::cosine_distance(emb1, emb2)
        } else {
            // Fallback to content-based comparison
            Self::content_similarity(&memory1.content, &memory2.content)
        };

        // Check if distance is in conflict range
        if !(CONFLICT_DISTANCE_MIN..=CONFLICT_DISTANCE_MAX).contains(&distance) {
            return None;
        }

        // Determine conflict type
        let conflict_type = Self::determine_conflict_type(memory1, memory2, distance);

        Some(MemoryConflict {
            id: Uuid::new_v4(),
            memory_ids: vec![memory1.id, memory2.id],
            conflicting_memories: vec![
                memory1.to_summary(),
                memory2.to_summary(),
            ],
            distance,
            conflict_type,
            detected_at: Utc::now(),
            repository: memory1.repository.clone(),
        })
    }

    /// Calculate cosine distance between two embeddings
    fn cosine_distance(embedding1: &[f32], embedding2: &[f32]) -> f32 {
        if embedding1.len() != embedding2.len() {
            return 1.0; // Different dimensions = maximum distance
        }

        let dot_product: f32 = embedding1
            .iter()
            .zip(embedding2.iter())
            .map(|(a, b)| a * b)
            .sum();

        let norm1: f32 = embedding1.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm2: f32 = embedding2.iter().map(|x| x * x).sum::<f32>().sqrt();

        if norm1 == 0.0 || norm2 == 0.0 {
            return 1.0; // Zero vector = maximum distance
        }

        let cosine_similarity = dot_product / (norm1 * norm2);
        (1.0 - cosine_similarity).clamp(0.0, 1.0)
    }

    /// Fallback content similarity when embeddings are not available
    fn content_similarity(content1: &str, content2: &str) -> f32 {
        let lower1 = content1.to_lowercase();
        let lower2 = content2.to_lowercase();
        let words1: std::collections::HashSet<_> = lower1.split_whitespace().collect();
        let words2: std::collections::HashSet<_> = lower2.split_whitespace().collect();

        if words1.is_empty() && words2.is_empty() {
            return 0.0;
        }

        let intersection = words1.intersection(&words2).count();
        let union = words1.union(&words2).count();

        if union == 0 {
            return 1.0;
        }

        // Jaccard distance = 1 - Jaccard similarity
        1.0 - (intersection as f32 / union as f32)
    }

    /// Determine the type of conflict between two memories
    fn determine_conflict_type(
        memory1: &ConflictMemoryInfo,
        memory2: &ConflictMemoryInfo,
        distance: f32,
    ) -> ConflictType {
        // Check for direct contradiction (content negation)
        if Self::is_contradiction(&memory1.content, &memory2.content) {
            return ConflictType::Contradiction;
        }

        // Check for confidence conflict
        let confidence_diff = (memory1.confidence - memory2.confidence).abs();
        if confidence_diff > 0.3 {
            return ConflictType::ConfidenceConflict;
        }

        // Check for outdated supersession (one is much newer)
        let age_diff = (memory1.updated_at - memory2.updated_at).num_days().abs();
        if age_diff > 30 && distance < 0.30 {
            return ConflictType::OutdatedSupersession;
        }

        // Check for provenance conflict (different sources)
        if memory1.source != memory2.source && distance < 0.35 {
            return ConflictType::ProvenanceConflict;
        }

        // Default to partial overlap
        ConflictType::PartialOverlap
    }

    /// Check if two contents are direct contradictions
    fn is_contradiction(content1: &str, content2: &str) -> bool {
        let contradictions = [
            ("always", "never"),
            ("all", "none"),
            ("yes", "no"),
            ("true", "false"),
            ("required", "optional"),
            ("must", "must not"),
            ("enable", "disable"),
            ("success", "failure"),
        ];

        let lower1 = content1.to_lowercase();
        let lower2 = content2.to_lowercase();

        for (positive, negative) in contradictions {
            if (lower1.contains(positive) && lower2.contains(negative))
                || (lower1.contains(negative) && lower2.contains(positive))
            {
                return true;
            }
        }

        false
    }

    /// Get the detector configuration
    pub fn config(&self) -> &ConflictResolutionConfig {
        &self.config
    }
}

impl Default for MemoryConflictDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// Information needed about a memory for conflict detection/resolution
#[derive(Debug, Clone)]
pub struct ConflictMemoryInfo {
    pub id: Uuid,
    pub label: String,
    pub content: String,
    pub embedding: Option<Vec<f32>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub confidence: f32,
    pub source: MemorySource,
    pub repository: String,
    pub metadata: serde_json::Value,
}

impl ConflictMemoryInfo {
    /// Convert to a conflict summary for logging
    pub fn to_summary(&self) -> MemoryConflictSummary {
        MemoryConflictSummary {
            id: self.id,
            label: self.label.clone(),
            content_preview: Self::truncate_content(&self.content, 100),
            created_at: self.created_at,
            updated_at: self.updated_at,
            confidence: self.confidence,
            source: self.source,
            is_operator_feedback: self.source == MemorySource::OperatorFeedback,
        }
    }

    fn truncate_content(content: &str, max_len: usize) -> String {
        if content.len() <= max_len {
            content.to_string()
        } else {
            format!("{}...", &content[..max_len.saturating_sub(3)])
        }
    }
}

/// Memory conflict resolver - applies resolution rules to determine winner
pub struct MemoryConflictResolver {
    detector: MemoryConflictDetector,
    config: ConflictResolutionConfig,
}

impl MemoryConflictResolver {
    /// Create a new conflict resolver with default configuration
    pub fn new() -> Self {
        Self {
            detector: MemoryConflictDetector::new(),
            config: ConflictResolutionConfig::default(),
        }
    }

    /// Create a new conflict resolver with custom configuration
    pub fn with_config(config: ConflictResolutionConfig) -> Self {
        Self {
            detector: MemoryConflictDetector::with_config(config.clone()),
            config,
        }
    }

    /// Resolve a detected conflict using the configured strategy
    pub fn resolve(&self, conflict: &MemoryConflict) -> ConflictResolutionResult {
        if conflict.conflicting_memories.len() < 2 {
            // Cannot resolve with less than 2 memories
            return self.create_unresolvable_result(conflict);
        }

        let memories = &conflict.conflicting_memories;
        let (winner_idx, loser_indices, strategy_used) = self.determine_winner(memories);

        let winner_id = memories[winner_idx].id;
        let loser_ids: Vec<Uuid> = loser_indices
            .iter()
            .map(|&idx| memories[idx].id)
            .collect();

        // Build loser memory references for explanation
        let loser_memories: Vec<&MemoryConflictSummary> = loser_indices
            .iter()
            .map(|&idx| &memories[idx])
            .collect();

        let resolution_reason = self.explain_resolution(
            &memories[winner_idx],
            &loser_memories,
            strategy_used,
        );

        if self.config.enable_logging {
            tracing::info!(
                conflict_id = %conflict.id,
                winner_id = %winner_id,
                loser_ids = ?loser_ids,
                strategy = %strategy_used,
                conflict_type = %conflict.conflict_type,
                distance = conflict.distance,
                "Memory conflict resolved"
            );
        }

        let superseded_ids = loser_ids.clone();

        ConflictResolutionResult {
            conflict: conflict.clone(),
            winner_id,
            loser_ids,
            strategy_used,
            resolution_reason,
            winner_updated: false,
            superseded_ids,
            resolved_at: Utc::now(),
        }
    }

    /// Resolve conflict between two specific memories
    pub fn resolve_pair(
        &self,
        memory1: &ConflictMemoryInfo,
        memory2: &ConflictMemoryInfo,
    ) -> Option<ConflictResolutionResult> {
        let conflict = self.detector.detect_conflict(memory1, memory2)?;
        Some(self.resolve(&conflict))
    }

    /// Determine which memory wins the conflict
    fn determine_winner(&self, memories: &[MemoryConflictSummary]) -> (usize, Vec<usize>, ResolutionStrategy) {
        // Strategy: Higher Confidence Wins (default)
        match self.config.primary_strategy {
            ResolutionStrategy::HigherConfidenceWins => {
                self.resolve_higher_confidence(memories)
            }
            ResolutionStrategy::NewerWins => {
                self.resolve_newer(memories)
            }
            ResolutionStrategy::OperatorOverrides => {
                self.resolve_operator_override(memories)
            }
            ResolutionStrategy::KeepOriginal => {
                self.resolve_keep_original(memories)
            }
            ResolutionStrategy::KeepNew => {
                self.resolve_keep_new(memories)
            }
            ResolutionStrategy::Merge | ResolutionStrategy::Manual => {
                // Fallback to higher confidence for merge/manual
                self.resolve_higher_confidence(memories)
            }
        }
    }

    /// Resolution: Higher confidence wins
    fn resolve_higher_confidence(&self, memories: &[MemoryConflictSummary]) -> (usize, Vec<usize>, ResolutionStrategy) {
        let mut winner_idx = 0;
        let mut max_confidence = memories[0].confidence;

        for (idx, memory) in memories.iter().enumerate().skip(1) {
            // Apply operator override if enabled
            if self.config.enable_operator_override && memory.is_operator_feedback {
                // Operator feedback wins regardless of confidence
                let (idx, losers, _) = self.resolve_operator_override(memories);
                return (idx, losers, ResolutionStrategy::OperatorOverrides);
            }

            if memory.confidence > max_confidence + self.config.confidence_difference_threshold {
                max_confidence = memory.confidence;
                winner_idx = idx;
            }
        }

        let loser_indices: Vec<usize> = (0..memories.len())
            .filter(|&i| i != winner_idx)
            .collect();

        (winner_idx, loser_indices, ResolutionStrategy::HigherConfidenceWins)
    }

    /// Resolution: Newer memory wins
    fn resolve_newer(&self, memories: &[MemoryConflictSummary]) -> (usize, Vec<usize>, ResolutionStrategy) {
        let mut winner_idx = 0;
        let mut newest_time = memories[0].updated_at;

        for (idx, memory) in memories.iter().enumerate().skip(1) {
            // Operator feedback wins if enabled
            if self.config.enable_operator_override && memory.is_operator_feedback {
                let (idx, losers, _) = self.resolve_operator_override(memories);
                return (idx, losers, ResolutionStrategy::OperatorOverrides);
            }

            if memory.updated_at > newest_time {
                newest_time = memory.updated_at;
                winner_idx = idx;
            }
        }

        let loser_indices: Vec<usize> = (0..memories.len())
            .filter(|&i| i != winner_idx)
            .collect();

        (winner_idx, loser_indices, ResolutionStrategy::NewerWins)
    }

    /// Resolution: Operator feedback always wins
    fn resolve_operator_override(
        &self,
        memories: &[MemoryConflictSummary],
    ) -> (usize, Vec<usize>, ResolutionStrategy) {
        // Find operator feedback memory
        for (idx, memory) in memories.iter().enumerate() {
            if memory.is_operator_feedback {
                let loser_indices: Vec<usize> = (0..memories.len())
                    .filter(|&i| i != idx)
                    .collect();
                return (idx, loser_indices, ResolutionStrategy::OperatorOverrides);
            }
        }

        // No operator feedback found, fall back to higher confidence
        self.resolve_higher_confidence(memories)
    }

    /// Resolution: Keep the original (first) memory
    fn resolve_keep_original(&self, memories: &[MemoryConflictSummary]) -> (usize, Vec<usize>, ResolutionStrategy) {
        let loser_indices: Vec<usize> = (1..memories.len()).collect();
        (0, loser_indices, ResolutionStrategy::KeepOriginal)
    }

    /// Resolution: Keep the newest memory
    fn resolve_keep_new(&self, memories: &[MemoryConflictSummary]) -> (usize, Vec<usize>, ResolutionStrategy) {
        let mut winner_idx = 0;
        let mut newest_time = memories[0].created_at;

        for (idx, memory) in memories.iter().enumerate().skip(1) {
            if memory.created_at > newest_time {
                newest_time = memory.created_at;
                winner_idx = idx;
            }
        }

        let loser_indices: Vec<usize> = (0..memories.len())
            .filter(|&i| i != winner_idx)
            .collect();

        (winner_idx, loser_indices, ResolutionStrategy::KeepNew)
    }

    /// Explain the resolution decision
    fn explain_resolution(
        &self,
        winner: &MemoryConflictSummary,
        losers: &[&MemoryConflictSummary],
        strategy: ResolutionStrategy,
    ) -> String {
        match strategy {
            ResolutionStrategy::HigherConfidenceWins => {
                format!(
                    "Winner has higher confidence ({:.2}) than loser(s) ({})",
                    winner.confidence,
                    losers
                        .iter()
                        .map(|m| format!("{:.2}", m.confidence))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            ResolutionStrategy::NewerWins => {
                format!(
                    "Winner is newer ({}) than loser(s) ({})",
                    winner.updated_at.to_rfc3339(),
                    losers
                        .iter()
                        .map(|m| m.updated_at.to_rfc3339())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            ResolutionStrategy::OperatorOverrides => {
                "Winner is operator-provided guidance which overrides learned knowledge".to_string()
            }
            ResolutionStrategy::KeepOriginal => {
                "Winner is the original memory, new memories were rejected".to_string()
            }
            ResolutionStrategy::KeepNew => {
                "Winner is the newest memory, older memories were superseded".to_string()
            }
            ResolutionStrategy::Merge => {
                "Content from all memories merged".to_string()
            }
            ResolutionStrategy::Manual => {
                "Conflict requires manual resolution".to_string()
            }
        }
    }

    /// Create result for unresolvable conflicts
    fn create_unresolvable_result(&self, conflict: &MemoryConflict) -> ConflictResolutionResult {
        ConflictResolutionResult {
            conflict: conflict.clone(),
            winner_id: Uuid::nil(),
            loser_ids: vec![],
            strategy_used: ResolutionStrategy::Manual,
            resolution_reason: "Conflict involves fewer than 2 memories - cannot resolve automatically".to_string(),
            winner_updated: false,
            superseded_ids: vec![],
            resolved_at: Utc::now(),
        }
    }

    /// Get the detector
    pub fn detector(&self) -> &MemoryConflictDetector {
        &self.detector
    }

    /// Get the configuration
    pub fn config(&self) -> &ConflictResolutionConfig {
        &self.config
    }
}

impl Default for MemoryConflictResolver {
    fn default() -> Self {
        Self::new()
    }
}

/// Service for managing memory conflicts and resolutions
pub struct ConflictResolutionService {
    resolver: MemoryConflictResolver,
}

impl ConflictResolutionService {
    /// Create a new conflict resolution service
    pub fn new() -> Self {
        Self {
            resolver: MemoryConflictResolver::new(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(config: ConflictResolutionConfig) -> Self {
        Self {
            resolver: MemoryConflictResolver::with_config(config),
        }
    }

    /// Detect conflicts between a new memory and existing memories
    pub async fn detect_conflicts(
        &self,
        new_memory: &ConflictMemoryInfo,
        existing_memories: &[ConflictMemoryInfo],
    ) -> Vec<MemoryConflict> {
        let mut conflicts = Vec::new();

        for existing in existing_memories {
            if let Some(conflict) = self.resolver.detector().detect_conflict(new_memory, existing) {
                conflicts.push(conflict);
            }
        }

        conflicts
    }

    /// Detect and resolve conflicts for a new memory against existing ones
    /// Returns the memories that should be superseded
    pub async fn detect_and_resolve(
        &self,
        new_memory: &ConflictMemoryInfo,
        existing_memories: &[ConflictMemoryInfo],
    ) -> Vec<ConflictResolutionResult> {
        let conflicts = self.detect_conflicts(new_memory, existing_memories).await;
        conflicts
            .iter()
            .map(|c| self.resolver.resolve(c))
            .collect()
    }

    /// Check if a new memory would conflict with any existing memories
    pub fn would_conflict(
        &self,
        new_memory: &ConflictMemoryInfo,
        existing_memories: &[ConflictMemoryInfo],
    ) -> bool {
        for existing in existing_memories {
            if self.resolver.detector().detect_conflict(new_memory, existing).is_some() {
                return true;
            }
        }
        false
    }

    /// Get the underlying resolver
    pub fn resolver(&self) -> &MemoryConflictResolver {
        &self.resolver
    }
}

impl Default for ConflictResolutionService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_memory(
        id: Uuid,
        label: &str,
        content: &str,
        confidence: f32,
        days_ago: i64,
        source: MemorySource,
    ) -> ConflictMemoryInfo {
        ConflictMemoryInfo {
            id,
            label: label.to_string(),
            content: content.to_string(),
            embedding: None,
            created_at: Utc::now() - chrono::Duration::days(days_ago),
            updated_at: Utc::now() - chrono::Duration::days(days_ago / 2),
            confidence,
            source,
            repository: "test-repo".to_string(),
            metadata: serde_json::json!({}),
        }
    }

    #[test]
    fn test_conflict_detection_same_label() {
        let detector = MemoryConflictDetector::new();

        let memory1 = create_test_memory(
            Uuid::new_v4(),
            "convention",
            "always run tests before commit",
            0.8,
            10,
            MemorySource::TaskSuccess,
        );

        let memory2 = create_test_memory(
            Uuid::new_v4(),
            "convention",
            "never run tests before commit",
            0.7,
            5,
            MemorySource::TaskFailure,
        );

        // Same label but different content - should detect potential conflict
        // Using content similarity fallback
        let conflict = detector.detect_conflict(&memory1, &memory2);
        assert!(conflict.is_some());

        let conflict = conflict.unwrap();
        assert_eq!(conflict.memory_ids.len(), 2);
        assert!(conflict.conflict_type == ConflictType::PartialOverlap
            || conflict.conflict_type == ConflictType::Contradiction);
    }

    #[test]
    fn test_conflict_detection_different_label() {
        let detector = MemoryConflictDetector::new();

        let memory1 = create_test_memory(
            Uuid::new_v4(),
            "convention-a",
            "always run tests",
            0.8,
            10,
            MemorySource::TaskSuccess,
        );

        let memory2 = create_test_memory(
            Uuid::new_v4(),
            "convention-b",
            "always run tests",
            0.7,
            5,
            MemorySource::TaskFailure,
        );

        // Different labels - no conflict possible
        let conflict = detector.detect_conflict(&memory1, &memory2);
        assert!(conflict.is_none());
    }

    #[test]
    fn test_conflict_detection_different_repository() {
        let detector = MemoryConflictDetector::new();

        let mut memory1 = create_test_memory(
            Uuid::new_v4(),
            "convention",
            "always run tests",
            0.8,
            10,
            MemorySource::TaskSuccess,
        );
        memory1.repository = "repo-a".to_string();

        let mut memory2 = create_test_memory(
            Uuid::new_v4(),
            "convention",
            "never run tests",
            0.7,
            5,
            MemorySource::TaskFailure,
        );
        memory2.repository = "repo-b".to_string();

        // Different repositories - no conflict
        let conflict = detector.detect_conflict(&memory1, &memory2);
        assert!(conflict.is_none());
    }

    #[test]
    fn test_resolution_higher_confidence_wins() {
        let resolver = MemoryConflictResolver::with_config(ConflictResolutionConfig {
            primary_strategy: ResolutionStrategy::HigherConfidenceWins,
            enable_operator_override: false,
            ..Default::default()
        });

        // Use similar but not identical content to ensure conflict detection
        let memory1 = create_test_memory(
            Uuid::new_v4(),
            "convention",
            "always run tests before commit",
            0.6,
            10,
            MemorySource::TaskSuccess,
        );

        let memory2 = create_test_memory(
            Uuid::new_v4(),
            "convention",
            "always run tests before commit and push",
            0.9,
            5,
            MemorySource::TaskSuccess,
        );

        let conflict = resolver.detector().detect_conflict(&memory1, &memory2);
        assert!(conflict.is_some(), "Similar content should be detected as conflict");

        let result = resolver.resolve(&conflict.unwrap());
        assert_eq!(result.winner_id, memory2.id);
        assert!(result.loser_ids.contains(&memory1.id));
        assert_eq!(result.strategy_used, ResolutionStrategy::HigherConfidenceWins);
    }

    #[test]
    fn test_resolution_newer_wins() {
        let resolver = MemoryConflictResolver::with_config(ConflictResolutionConfig {
            primary_strategy: ResolutionStrategy::NewerWins,
            enable_operator_override: false,
            ..Default::default()
        });

        // Use similar but not identical content to ensure conflict detection
        let memory1 = create_test_memory(
            Uuid::new_v4(),
            "convention",
            "always run tests before commit",
            0.9,
            30, // Created 30 days ago
            MemorySource::TaskSuccess,
        );

        let memory2 = create_test_memory(
            Uuid::new_v4(),
            "convention",
            "always run tests before commit and push",
            0.5, // Lower confidence
            1,   // Created 1 day ago
            MemorySource::TaskSuccess,
        );

        let conflict = resolver.detector().detect_conflict(&memory1, &memory2);
        assert!(conflict.is_some(), "Similar content should be detected as conflict");

        let result = resolver.resolve(&conflict.unwrap());
        assert_eq!(result.winner_id, memory2.id);
        assert_eq!(result.strategy_used, ResolutionStrategy::NewerWins);
    }

    #[test]
    fn test_resolution_operator_override() {
        let resolver = MemoryConflictResolver::with_config(ConflictResolutionConfig {
            primary_strategy: ResolutionStrategy::HigherConfidenceWins,
            enable_operator_override: true,
            ..Default::default()
        });

        // Use similar but not identical content to ensure conflict detection
        // Agent-generated high confidence memory
        let agent_memory = create_test_memory(
            Uuid::new_v4(),
            "convention",
            "always run tests before commit",
            0.95, // Very high confidence
            1,
            MemorySource::TaskSuccess,
        );

        // Operator feedback memory (lower confidence but should win)
        let operator_memory = create_test_memory(
            Uuid::new_v4(),
            "convention",
            "always run tests before commit and push",
            0.85, // Lower confidence but operator wins
            2,
            MemorySource::OperatorFeedback,
        );

        let conflict = resolver.detector().detect_conflict(&agent_memory, &operator_memory);
        assert!(conflict.is_some(), "Similar content should be detected as conflict");

        let result = resolver.resolve(&conflict.unwrap());
        assert_eq!(result.winner_id, operator_memory.id);
        // When operator override is triggered, strategy_used should be OperatorOverrides
        assert_eq!(result.strategy_used, ResolutionStrategy::OperatorOverrides);
        assert!(result.resolution_reason.contains("operator-provided"));
    }

    #[test]
    fn test_is_contradiction() {
        let contradictions = [
            ("always do X", "never do X"),
            ("all tests pass", "none of the tests pass"),
            ("success is expected", "failure is expected"),
            ("required field", "optional field"),
        ];

        for (positive, negative) in contradictions {
            assert!(
                MemoryConflictDetector::is_contradiction(positive, negative),
                "Expected '{}' and '{}' to be detected as contradiction",
                positive,
                negative
            );
        }

        let non_contradictions = [
            ("run tests", "run lint"),
            ("fix bug", "add feature"),
            ("update README", "update config"),
        ];

        for (a, b) in non_contradictions {
            assert!(
                !MemoryConflictDetector::is_contradiction(a, b),
                "Expected '{}' and '{}' NOT to be detected as contradiction",
                a,
                b
            );
        }
    }

    #[test]
    fn test_content_similarity() {
        // Identical content
        let dist = MemoryConflictDetector::content_similarity("hello world", "hello world");
        assert!(dist < 0.01, "Identical content should have near-zero distance");

        // Completely different content
        let dist = MemoryConflictDetector::content_similarity("cat dog bird", "red green blue");
        assert!(dist > 0.9, "Completely different content should have high distance");

        // Partial overlap
        let dist = MemoryConflictDetector::content_similarity(
            "run tests before commit",
            "run lint before commit",
        );
        assert!(dist > 0.3 && dist < 0.7, "Partial overlap should have moderate distance");
    }

    #[test]
    fn test_conflict_resolution_log_entry() {
        let memory1 = create_test_memory(
            Uuid::new_v4(),
            "convention",
            "content 1",
            0.8,
            10,
            MemorySource::TaskSuccess,
        );

        let memory2 = create_test_memory(
            Uuid::new_v4(),
            "convention",
            "content 2",
            0.7,
            5,
            MemorySource::TaskFailure,
        );

        let conflict = MemoryConflict {
            id: Uuid::new_v4(),
            memory_ids: vec![memory1.id, memory2.id],
            conflicting_memories: vec![memory1.to_summary(), memory2.to_summary()],
            distance: 0.25,
            conflict_type: ConflictType::PartialOverlap,
            detected_at: Utc::now(),
            repository: "test-repo".to_string(),
        };

        let resolver = MemoryConflictResolver::new();
        let result = resolver.resolve(&conflict);

        let log_entry = ConflictResolutionLogEntry::from_result(&result, "test-repo".to_string());

        assert_eq!(log_entry.conflict_id, conflict.id);
        assert_eq!(log_entry.winner_id, result.winner_id);
        assert_eq!(log_entry.loser_ids, result.loser_ids);
        assert_eq!(log_entry.strategy, result.strategy_used);
        assert_eq!(log_entry.repository, "test-repo");
    }

    #[test]
    fn test_conflict_resolution_config_default() {
        let config = ConflictResolutionConfig::default();

        assert_eq!(config.primary_strategy, ResolutionStrategy::HigherConfidenceWins);
        assert_eq!(config.fallback_strategy, ResolutionStrategy::NewerWins);
        assert!(config.enable_operator_override);
        assert!(config.enable_logging);
    }

    #[test]
    fn test_memory_source_outranks() {
        // Operator feedback outranks everything except itself
        assert!(!MemorySource::OperatorFeedback.outranks(&MemorySource::OperatorFeedback), "Same source should not outrank itself");
        assert!(MemorySource::OperatorFeedback.outranks(&MemorySource::TaskSuccess));
        assert!(MemorySource::OperatorFeedback.outranks(&MemorySource::TaskFailure));
        assert!(MemorySource::OperatorFeedback.outranks(&MemorySource::PatternLearning));

        // Task success outranks task failure
        assert!(MemorySource::TaskSuccess.outranks(&MemorySource::TaskFailure));
        assert!(!MemorySource::TaskFailure.outranks(&MemorySource::TaskSuccess));

        // Manual outranks learned
        assert!(MemorySource::Manual.outranks(&MemorySource::PatternLearning));
        assert!(MemorySource::Manual.outranks(&MemorySource::SkillExtraction));

        // Same tier comparisons
        assert!(!MemorySource::TaskSuccess.outranks(&MemorySource::TaskSuccess), "Same source should not outrank itself");
        assert!(!MemorySource::Manual.outranks(&MemorySource::Manual), "Same source should not outrank itself");
    }

    #[test]
    fn test_resolution_strategy_display() {
        assert_eq!(ResolutionStrategy::HigherConfidenceWins.to_string(), "higher_confidence_wins");
        assert_eq!(ResolutionStrategy::NewerWins.to_string(), "newer_wins");
        assert_eq!(ResolutionStrategy::OperatorOverrides.to_string(), "operator_overrides");
    }

    #[test]
    fn test_conflict_type_display() {
        assert_eq!(ConflictType::Contradiction.to_string(), "contradiction");
        assert_eq!(ConflictType::OutdatedSupersession.to_string(), "outdated_supersession");
        assert_eq!(ConflictType::PartialOverlap.to_string(), "partial_overlap");
    }

    #[test]
    fn test_would_conflict() {
        let service = ConflictResolutionService::new();

        let existing = vec![create_test_memory(
            Uuid::new_v4(),
            "convention",
            "always run tests before commit",
            0.8,
            10,
            MemorySource::TaskSuccess,
        )];

        // Very similar content should conflict
        let new_similar = create_test_memory(
            Uuid::new_v4(),
            "convention",
            "always run tests before commit and push",
            0.7,
            5,
            MemorySource::TaskFailure,
        );
        assert!(service.would_conflict(&new_similar, &existing), "Similar content should conflict");

        // Different label should not conflict
        let new_different_label = create_test_memory(
            Uuid::new_v4(),
            "different-label",
            "always run tests before commit",
            0.7,
            5,
            MemorySource::TaskFailure,
        );
        assert!(!service.would_conflict(&new_different_label, &existing));
    }

    #[test]
    fn test_conflict_memory_info_to_summary() {
        let memory = create_test_memory(
            Uuid::new_v4(),
            "test-label",
            "short content",
            0.8,
            10,
            MemorySource::TaskSuccess,
        );

        let summary = memory.to_summary();

        assert_eq!(summary.id, memory.id);
        assert_eq!(summary.label, memory.label);
        assert_eq!(summary.confidence, memory.confidence);
        assert_eq!(summary.source, memory.source);
        assert!(!summary.is_operator_feedback);
    }

    #[test]
    fn test_truncate_content() {
        let short = "short";
        assert_eq!(ConflictMemoryInfo::truncate_content(short, 100), "short");

        let long = "this is a very long piece of content that should be truncated";
        let truncated = ConflictMemoryInfo::truncate_content(long, 20);
        assert!(truncated.ends_with("..."));
        assert!(truncated.len() <= 20);
    }

    #[tokio::test]
    async fn test_detect_and_resolve() {
        let service = ConflictResolutionService::new();

        // Use similar but not identical content to ensure conflict detection
        let existing = vec![create_test_memory(
            Uuid::new_v4(),
            "convention",
            "always run tests before commit",
            0.6,
            30,
            MemorySource::TaskSuccess,
        )];

        let new_memory = create_test_memory(
            Uuid::new_v4(),
            "convention",
            "always run tests before commit and push",
            0.9,
            1,
            MemorySource::TaskSuccess,
        );

        let results = service.detect_and_resolve(&new_memory, &existing).await;
        assert_eq!(results.len(), 1, "Should detect one conflict");

        let result = &results[0];
        assert_eq!(result.winner_id, new_memory.id);
        assert!(result.loser_ids.contains(&existing[0].id));
    }

    #[test]
    fn test_conflict_distance_constants() {
        assert!(CONFLICT_DISTANCE_MIN < CONFLICT_DISTANCE_MAX);
        assert!(CONFLICT_DISTANCE_MIN > 0.0);
        assert!(CONFLICT_DISTANCE_MAX < 1.0);
    }

    #[test]
    fn test_low_confidence_threshold() {
        assert!(LOW_CONFIDENCE_THRESHOLD > 0.0);
        assert!(LOW_CONFIDENCE_THRESHOLD < 1.0);
        assert!(LOW_CONFIDENCE_THRESHOLD < 0.6); // Should be below typical confidence
    }
}
