use crate::{SqliteMemoryStore, SwellError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use swell_core::{ids::TaskId, MemoryStore};
use uuid::Uuid;

/// A learned pattern extracted from a task trajectory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearnedPattern {
    /// Unique identifier for this pattern
    pub id: Uuid,
    /// Human-readable name for the pattern
    pub name: String,
    /// The sequence of tool names that form this pattern
    pub tool_sequence: Vec<String>,
    /// Number of times this pattern was observed
    pub observation_count: u32,
    /// Confidence score (0.0 to 1.0) based on observation frequency and consistency
    pub confidence: f64,
    /// Whether this pattern has been validated and is ready for reuse
    pub is_validated: bool,
    /// Source task ID where this pattern was first observed
    pub source_task_id: TaskId,
    /// Repository where this pattern was learned
    pub repository: String,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
    /// Last updated timestamp
    pub updated_at: DateTime<Utc>,
    /// Optional description of what this pattern accomplishes
    pub description: Option<String>,
    /// Metadata for additional pattern information
    pub metadata: serde_json::Value,
}

impl LearnedPattern {
    /// Create a new learned pattern from a tool sequence
    pub fn new(tool_sequence: Vec<String>, source_task_id: TaskId, repository: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: Self::generate_pattern_name(&tool_sequence),
            tool_sequence,
            observation_count: 1,
            confidence: 0.0,
            is_validated: false,
            source_task_id,
            repository,
            created_at: now,
            updated_at: now,
            description: None,
            metadata: serde_json::json!({}),
        }
    }

    /// Generate a descriptive name from the tool sequence
    fn generate_pattern_name(sequence: &[String]) -> String {
        if sequence.is_empty() {
            return "empty_pattern".to_string();
        }

        let tools_str = sequence.join("_");
        // Truncate if too long
        if tools_str.len() > 50 {
            format!("{}...pattern", &tools_str[..47])
        } else {
            format!("{}...pattern", tools_str)
        }
    }

    /// Update the confidence score based on observation count
    /// Uses a formula that rewards consistent patterns but caps at 0.95
    pub fn update_confidence(&mut self) {
        // Confidence formula: base confidence increases with observations
        // but plateaus to avoid overconfidence
        let base_confidence = (self.observation_count as f64 / 10.0).min(1.0);
        // Add a small boost for validated patterns
        let validated_boost = if self.is_validated { 0.1 } else { 0.0 };
        self.confidence = (base_confidence + validated_boost).min(0.95);
        self.updated_at = Utc::now();
    }

    /// Record an additional observation of this pattern
    pub fn record_observation(&mut self) {
        self.observation_count += 1;
        self.update_confidence();
    }

    /// Mark pattern as validated
    pub fn validate(&mut self) {
        self.is_validated = true;
        self.update_confidence();
    }
}

/// A tool call record from a task trajectory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub success: bool,
    pub timestamp: DateTime<Utc>,
}

/// A complete task trajectory for analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskTrajectory {
    pub task_id: TaskId,
    pub task_description: String,
    pub tool_calls: Vec<ToolCallRecord>,
    pub outcome: TaskOutcome,
    pub iteration_count: u32,
    pub repository: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    Success,
    Failure,
    Cancelled,
}

/// Result of post-session pattern learning analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostSessionLearningResult {
    pub patterns_extracted: usize,
    pub patterns: Vec<LearnedPattern>,
    pub errors: Vec<String>,
}

/// Configuration for post-session learning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostSessionLearningConfig {
    /// Minimum observation count before a pattern is considered
    pub min_observations: u32,
    /// Minimum confidence threshold for storing patterns
    pub min_confidence: f64,
    /// Maximum number of patterns to store per session
    pub max_patterns_per_session: usize,
    /// Minimum sequence length to consider (in tools)
    pub min_sequence_length: usize,
    /// Maximum sequence length to consider (in tools)
    pub max_sequence_length: usize,
}

impl Default for PostSessionLearningConfig {
    fn default() -> Self {
        Self {
            min_observations: 1,
            min_confidence: 0.15,
            max_patterns_per_session: 10,
            min_sequence_length: 2,
            max_sequence_length: 5,
        }
    }
}

/// Post-session pattern learner
pub struct PostSessionLearner {
    store: SqliteMemoryStore,
    config: PostSessionLearningConfig,
}

impl PostSessionLearner {
    /// Create a new post-session learner
    pub fn new(store: SqliteMemoryStore, config: PostSessionLearningConfig) -> Self {
        Self { store, config }
    }

    /// Create with default configuration
    pub fn with_default_config(store: SqliteMemoryStore) -> Self {
        Self {
            store,
            config: PostSessionLearningConfig::default(),
        }
    }

    /// Learn patterns from a completed task trajectory
    /// Extracts repeating tool sequences and stores them as learned patterns
    pub async fn learn_from_trajectory(
        &self,
        trajectory: TaskTrajectory,
    ) -> Result<PostSessionLearningResult, SwellError> {
        let mut errors = Vec::new();

        // Only learn from successful tasks
        if trajectory.outcome != TaskOutcome::Success {
            return Ok(PostSessionLearningResult {
                patterns_extracted: 0,
                patterns: Vec::new(),
                errors: vec!["Cannot learn from failed or cancelled tasks".to_string()],
            });
        }

        // Extract tool sequences from trajectory
        let sequences = self.extract_sequences(&trajectory.tool_calls);

        if sequences.is_empty() {
            return Ok(PostSessionLearningResult {
                patterns_extracted: 0,
                patterns: Vec::new(),
                errors: Vec::new(),
            });
        }

        // Count sequence occurrences
        let sequence_counts = self.count_sequences(&sequences);

        // Filter by minimum observations and confidence
        let patterns = self.build_patterns(sequence_counts, &trajectory);

        let pattern_count = patterns.len();

        // Store patterns in memory
        for pattern in &patterns {
            if let Err(e) = self.store_pattern(pattern).await {
                errors.push(format!("Failed to store pattern {}: {}", pattern.name, e));
            }
        }

        Ok(PostSessionLearningResult {
            patterns_extracted: pattern_count,
            patterns,
            errors,
        })
    }

    /// Extract tool sequences from tool calls
    fn extract_sequences(&self, tool_calls: &[ToolCallRecord]) -> Vec<Vec<String>> {
        let mut sequences = Vec::new();

        // Extract sequences of varying lengths
        for length in self.config.min_sequence_length..=self.config.max_sequence_length {
            for window in tool_calls.windows(length) {
                // Only consider sequences of successful calls
                if window.iter().all(|tc| tc.success) {
                    let sequence: Vec<String> =
                        window.iter().map(|tc| tc.tool_name.clone()).collect();
                    sequences.push(sequence);
                }
            }
        }

        sequences
    }

    /// Count occurrences of each unique sequence
    fn count_sequences(&self, sequences: &[Vec<String>]) -> HashMap<Vec<String>, usize> {
        let mut counts: HashMap<Vec<String>, usize> = HashMap::new();

        for sequence in sequences {
            *counts.entry(sequence.clone()).or_insert(0) += 1;
        }

        counts
    }

    /// Build learned patterns from sequence counts
    fn build_patterns(
        &self,
        sequence_counts: HashMap<Vec<String>, usize>,
        trajectory: &TaskTrajectory,
    ) -> Vec<LearnedPattern> {
        let mut patterns = Vec::new();

        // Sort by count (descending) to get most frequent patterns first
        let mut sorted: Vec<_> = sequence_counts.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));

        // Take top patterns up to max_patterns_per_session
        for (sequence, count) in sorted
            .into_iter()
            .take(self.config.max_patterns_per_session)
        {
            // Skip sequences with insufficient observations
            if count < self.config.min_observations as usize {
                continue;
            }

            let mut pattern = LearnedPattern::new(
                sequence.clone(),
                trajectory.task_id,
                trajectory.repository.clone(),
            );
            pattern.observation_count = count as u32;
            pattern.update_confidence();

            // Only include patterns meeting confidence threshold
            if pattern.confidence >= self.config.min_confidence {
                patterns.push(pattern);
            }
        }

        patterns
    }

    /// Store a learned pattern in memory
    async fn store_pattern(&self, pattern: &LearnedPattern) -> Result<(), SwellError> {
        let entry = crate::MemoryEntry {
            id: pattern.id,
            block_type: swell_core::MemoryBlockType::Skill,
            label: format!("pattern:{}", pattern.name),
            content: serde_json::to_string(pattern).unwrap_or_default(),
            embedding: None,
            created_at: pattern.created_at,
            updated_at: pattern.updated_at,
            metadata: serde_json::json!({
                "pattern_type": "post_session_learned",
                "confidence": pattern.confidence,
                "tool_sequence": &pattern.tool_sequence,
            }),
            // Full scope hierarchy
            org: String::new(),
            workspace: String::new(),
            repository: pattern.repository.clone(),
            language: None,
            framework: None,
            environment: None,
            task_type: None,
            session_id: None,
            last_reinforcement: Some(Utc::now()),
            is_stale: false,
            source_episode_id: Some(pattern.source_task_id.as_uuid()),
            evidence: None,
            provenance_context: None,
        };

        self.store
            .store(entry)
            .await
            .map_err(|e| SwellError::DatabaseError(e.to_string()))?;
        Ok(())
    }

    /// Retrieve learned patterns for a repository
    pub async fn get_patterns(&self, repository: &str) -> Result<Vec<LearnedPattern>, SwellError> {
        let entries = self
            .store
            .get_by_label("pattern:".to_string(), repository.to_string())
            .await?;

        let mut patterns = Vec::new();
        for entry in entries {
            // Only parse entries that are post-session learned patterns
            if let Some(label) = entry.label.strip_prefix("pattern:") {
                if label.ends_with("...pattern") {
                    if let Ok(pattern) = serde_json::from_str::<LearnedPattern>(&entry.content) {
                        patterns.push(pattern);
                    }
                }
            }
        }

        patterns.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(patterns)
    }

    /// Update an existing pattern with new observation
    pub async fn reinforce_pattern(&self, pattern_id: Uuid) -> Result<(), SwellError> {
        // Find the pattern entry
        if let Some(mut entry) = self.store.get(pattern_id).await? {
            // Parse the pattern
            if let Ok(mut pattern) = serde_json::from_str::<LearnedPattern>(&entry.content) {
                pattern.record_observation();
                entry.content = serde_json::to_string(&pattern).unwrap_or_default();
                entry.updated_at = Utc::now();
                entry.metadata = serde_json::json!({
                    "pattern_type": "post_session_learned",
                    "confidence": pattern.confidence,
                    "tool_sequence": &pattern.tool_sequence,
                });
                self.store.update(entry).await?;
            }
        }
        Ok(())
    }
}

/// High-level service for post-session learning
pub struct PostSessionLearningService {
    learner: PostSessionLearner,
}

impl PostSessionLearningService {
    /// Create a new post-session learning service
    pub fn new(store: SqliteMemoryStore, config: PostSessionLearningConfig) -> Self {
        Self {
            learner: PostSessionLearner::new(store, config),
        }
    }

    /// Create with default configuration
    pub fn with_default_config(store: SqliteMemoryStore) -> Self {
        Self {
            learner: PostSessionLearner::with_default_config(store),
        }
    }

    /// Analyze a completed task trajectory and learn patterns from it
    pub async fn analyze_and_learn(
        &self,
        trajectory: TaskTrajectory,
    ) -> Result<PostSessionLearningResult, SwellError> {
        self.learner.learn_from_trajectory(trajectory).await
    }

    /// Get all learned patterns for a repository
    pub async fn get_learned_patterns(
        &self,
        repository: &str,
    ) -> Result<Vec<LearnedPattern>, SwellError> {
        self.learner.get_patterns(repository).await
    }

    /// Reinforce a pattern after observing it again
    pub async fn reinforce(&self, pattern_id: Uuid) -> Result<(), SwellError> {
        self.learner.reinforce_pattern(pattern_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_trajectory(outcome: TaskOutcome) -> TaskTrajectory {
        TaskTrajectory {
            task_id: TaskId::new(),
            task_description: "Test task".to_string(),
            tool_calls: vec![
                ToolCallRecord {
                    tool_name: "read_file".to_string(),
                    arguments: serde_json::json!({"path": "src/main.rs"}),
                    success: true,
                    timestamp: Utc::now(),
                },
                ToolCallRecord {
                    tool_name: "edit_file".to_string(),
                    arguments: serde_json::json!({"path": "src/main.rs"}),
                    success: true,
                    timestamp: Utc::now(),
                },
                ToolCallRecord {
                    tool_name: "read_file".to_string(),
                    arguments: serde_json::json!({"path": "src/lib.rs"}),
                    success: true,
                    timestamp: Utc::now(),
                },
                ToolCallRecord {
                    tool_name: "edit_file".to_string(),
                    arguments: serde_json::json!({"path": "src/lib.rs"}),
                    success: true,
                    timestamp: Utc::now(),
                },
                ToolCallRecord {
                    tool_name: "shell".to_string(),
                    arguments: serde_json::json!({"command": "cargo build"}),
                    success: true,
                    timestamp: Utc::now(),
                },
            ],
            outcome,
            iteration_count: 1,
            repository: "test-repo".to_string(),
        }
    }

    #[test]
    fn test_learned_pattern_creation() {
        let sequence = vec!["read_file".to_string(), "edit_file".to_string()];
        let pattern =
            LearnedPattern::new(sequence.clone(), TaskId::new(), "test-repo".to_string());

        assert_eq!(pattern.tool_sequence, sequence);
        assert_eq!(pattern.observation_count, 1);
        assert_eq!(pattern.confidence, 0.0); // Initially 0, updated later
        assert!(!pattern.is_validated);
    }

    #[test]
    fn test_pattern_name_generation() {
        let short_seq = vec!["read_file".to_string()];
        let pattern = LearnedPattern::new(short_seq, TaskId::new(), "test-repo".to_string());
        assert!(pattern.name.contains("read_file"));

        let long_seq = vec![
            "read_file".to_string(),
            "edit_file".to_string(),
            "shell".to_string(),
            "read_file".to_string(),
            "edit_file".to_string(),
        ];
        let pattern2 = LearnedPattern::new(long_seq, TaskId::new(), "test-repo".to_string());
        assert!(pattern2.name.len() <= 100); // Allow room for longer names with truncation
    }

    #[test]
    fn test_pattern_confidence_update() {
        let mut pattern = LearnedPattern::new(
            vec!["read_file".to_string()],
            Uuid::new_v4(),
            "test-repo".to_string(),
        );

        // After one observation, confidence should be low
        pattern.update_confidence();
        assert!(pattern.confidence < 0.2);

        // After multiple observations, confidence should increase
        pattern.observation_count = 5;
        pattern.update_confidence();
        assert!(pattern.confidence > 0.3);

        // Capped at 0.95
        pattern.observation_count = 100;
        pattern.update_confidence();
        assert_eq!(pattern.confidence, 0.95);
    }

    #[test]
    fn test_pattern_record_observation() {
        let mut pattern = LearnedPattern::new(
            vec!["shell".to_string()],
            Uuid::new_v4(),
            "test-repo".to_string(),
        );

        let initial_count = pattern.observation_count;
        pattern.record_observation();
        assert_eq!(pattern.observation_count, initial_count + 1);
    }

    #[test]
    fn test_pattern_validation() {
        let mut pattern = LearnedPattern::new(
            vec!["shell".to_string()],
            Uuid::new_v4(),
            "test-repo".to_string(),
        );

        assert!(!pattern.is_validated);
        pattern.validate();
        assert!(pattern.is_validated);
    }

    #[tokio::test]
    async fn test_post_session_learner_config_default() {
        let config = PostSessionLearningConfig::default();
        assert_eq!(config.min_observations, 1);
        assert_eq!(config.min_confidence, 0.15);
        assert_eq!(config.max_patterns_per_session, 10);
        assert_eq!(config.min_sequence_length, 2);
        assert_eq!(config.max_sequence_length, 5);
    }

    #[tokio::test]
    async fn test_extract_sequences() {
        let store = crate::SqliteMemoryStore::create("sqlite::memory:")
            .await
            .unwrap();
        let learner = PostSessionLearner::with_default_config(store);

        let tool_calls = vec![
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
            ToolCallRecord {
                tool_name: "shell".to_string(),
                arguments: serde_json::json!({}),
                success: true,
                timestamp: Utc::now(),
            },
        ];

        let sequences = learner.extract_sequences(&tool_calls);

        // Should have sequences of length 2, 3, 4, 5 (up to max_sequence_length=5)
        // For length 2: read_file+edit_file, edit_file+shell (2 sequences)
        // For length 3: read_file+edit_file+shell (1 sequence)
        assert!(sequences.len() >= 3);

        // Check that our expected length-2 sequences exist
        let has_read_edit = sequences
            .iter()
            .any(|s| s == &vec!["read_file", "edit_file"]);
        let has_edit_shell = sequences.iter().any(|s| s == &vec!["edit_file", "shell"]);
        assert!(has_read_edit);
        assert!(has_edit_shell);
    }

    #[tokio::test]
    async fn test_extract_sequences_excludes_failed() {
        let store = crate::SqliteMemoryStore::create("sqlite::memory:")
            .await
            .unwrap();
        let learner = PostSessionLearner::with_default_config(store);

        let tool_calls = vec![
            ToolCallRecord {
                tool_name: "read_file".to_string(),
                arguments: serde_json::json!({}),
                success: true,
                timestamp: Utc::now(),
            },
            ToolCallRecord {
                tool_name: "edit_file".to_string(),
                arguments: serde_json::json!({}),
                success: false, // Failed call
                timestamp: Utc::now(),
            },
            ToolCallRecord {
                tool_name: "shell".to_string(),
                arguments: serde_json::json!({}),
                success: true,
                timestamp: Utc::now(),
            },
        ];

        let sequences = learner.extract_sequences(&tool_calls);

        // Should not have edit_file in any sequence since it failed
        for sequence in &sequences {
            assert!(!sequence.contains(&"edit_file".to_string()));
        }
    }

    #[tokio::test]
    async fn test_count_sequences() {
        let store = crate::SqliteMemoryStore::create("sqlite::memory:")
            .await
            .unwrap();
        let learner = PostSessionLearner::with_default_config(store);

        let sequences = vec![
            vec!["read_file".to_string(), "edit_file".to_string()],
            vec!["read_file".to_string(), "edit_file".to_string()],
            vec!["read_file".to_string(), "edit_file".to_string()],
            vec!["edit_file".to_string(), "shell".to_string()],
        ];

        let counts = learner.count_sequences(&sequences);

        assert_eq!(
            counts.get(&vec!["read_file".to_string(), "edit_file".to_string()]),
            Some(&3)
        );
        assert_eq!(
            counts.get(&vec!["edit_file".to_string(), "shell".to_string()]),
            Some(&1)
        );
    }

    #[tokio::test]
    async fn test_build_patterns() {
        let store = crate::SqliteMemoryStore::create("sqlite::memory:")
            .await
            .unwrap();
        let learner = PostSessionLearner::with_default_config(store);

        let trajectory = create_test_trajectory(TaskOutcome::Success);
        let sequences = learner.extract_sequences(&trajectory.tool_calls);
        let counts = learner.count_sequences(&sequences);
        let patterns = learner.build_patterns(counts, &trajectory);

        // Should have extracted some patterns
        assert!(!patterns.is_empty());

        // All patterns should meet confidence threshold (0.15 is min_confidence default)
        for pattern in &patterns {
            assert!(pattern.confidence >= 0.15);
        }
    }

    #[tokio::test]
    async fn test_learn_from_successful_trajectory() {
        let store = crate::SqliteMemoryStore::create("sqlite::memory:")
            .await
            .unwrap();
        let service = PostSessionLearningService::with_default_config(store);

        let trajectory = create_test_trajectory(TaskOutcome::Success);
        let result = service.analyze_and_learn(trajectory).await.unwrap();

        // Should have extracted patterns from successful task
        assert!(result.patterns_extracted > 0);
        assert!(result.errors.is_empty());
    }

    #[tokio::test]
    async fn test_learn_from_failed_trajectory() {
        let store = crate::SqliteMemoryStore::create("sqlite::memory:")
            .await
            .unwrap();
        let service = PostSessionLearningService::with_default_config(store);

        let trajectory = create_test_trajectory(TaskOutcome::Failure);
        let result = service.analyze_and_learn(trajectory).await.unwrap();

        // Should not extract patterns from failed task
        assert_eq!(result.patterns_extracted, 0);
        assert!(!result.errors.is_empty() || result.patterns.is_empty());
    }

    #[tokio::test]
    async fn test_learned_pattern_serialization() {
        let mut pattern = LearnedPattern::new(
            vec!["read_file".to_string(), "edit_file".to_string()],
            Uuid::new_v4(),
            "test-repo".to_string(),
        );
        pattern.description = Some("Read and modify files".to_string());

        let json = serde_json::to_string(&pattern).unwrap();
        let deserialized: LearnedPattern = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.tool_sequence, pattern.tool_sequence);
        assert_eq!(deserialized.description, pattern.description);
    }

    #[tokio::test]
    async fn test_task_trajectory_serialization() {
        let trajectory = create_test_trajectory(TaskOutcome::Success);

        let json = serde_json::to_string(&trajectory).unwrap();
        let deserialized: TaskTrajectory = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.task_id, trajectory.task_id);
        assert_eq!(deserialized.outcome, TaskOutcome::Success);
        assert_eq!(deserialized.tool_calls.len(), trajectory.tool_calls.len());
    }

    #[tokio::test]
    async fn test_tool_call_record_serialization() {
        let record = ToolCallRecord {
            tool_name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo build"}),
            success: true,
            timestamp: Utc::now(),
        };

        let json = serde_json::to_string(&record).unwrap();
        let deserialized: ToolCallRecord = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.tool_name, "shell");
        assert!(deserialized.success);
    }

    #[tokio::test]
    async fn test_post_session_learning_result() {
        let result = PostSessionLearningResult {
            patterns_extracted: 3,
            patterns: Vec::new(),
            errors: vec!["Error 1".to_string()],
        };

        assert_eq!(result.patterns_extracted, 3);
        assert_eq!(result.errors.len(), 1);
    }

    #[tokio::test]
    async fn test_pattern_confidence_with_validation_boost() {
        let mut pattern = LearnedPattern::new(
            vec!["shell".to_string()],
            Uuid::new_v4(),
            "test-repo".to_string(),
        );

        pattern.observation_count = 5;
        pattern.update_confidence();
        let unvalidated_confidence = pattern.confidence;

        pattern.validate();
        let validated_confidence = pattern.confidence;

        // Validated patterns should have higher confidence
        assert!(validated_confidence > unvalidated_confidence);
    }
}
