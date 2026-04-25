// cross_encoder_rerank.rs - Cross-encoder reranking for memory retrieval using BGE-reranker
//
// This module provides cross-encoder reranking to improve retrieval quality.
// Cross-encoders jointly encode query-document pairs, providing more accurate
// relevance scoring than the separate encoding used in bi-encoders.
//
// The BGE-reranker-v2-m3 model (278M parameters) provides state-of-the-art
// reranking performance for code and text retrieval tasks.

use crate::{MemoryEntry, SwellError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

/// Configuration for cross-encoder reranking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossEncoderConfig {
    /// Whether reranking is enabled
    pub enabled: bool,
    /// Maximum number of candidates to rerank
    pub max_candidates: usize,
    /// Maximum number of results to return after reranking
    pub max_results: usize,
    /// Model name for BGE reranker (used when model_type is "bge")
    pub model_name: Option<String>,
    /// Model type: "bge", "mock", or "simple"
    pub model_type: RerankerModelType,
    /// ONNX model path for local BGE reranker (optional)
    pub onnx_model_path: Option<String>,
    /// Whether to truncate documents that exceed max token length
    pub truncate_documents: bool,
    /// Maximum token length for documents
    pub max_doc_length: usize,
    /// Batch size for reranking multiple candidates
    pub batch_size: usize,
}

impl Default for CrossEncoderConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_candidates: 50,
            max_results: 10,
            model_name: Some("BAAI/bge-reranker-v2-m3".to_string()),
            model_type: RerankerModelType::Simple,
            onnx_model_path: None,
            truncate_documents: true,
            max_doc_length: 512,
            batch_size: 8,
        }
    }
}

/// Type of reranker model
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RerankerModelType {
    /// BGE reranker with ONNX runtime (requires onnxruntime package)
    Bge,
    /// Simple keyword-based reranker for testing/MVP
    #[default]
    Simple,
    /// Mock reranker that returns deterministic scores
    Mock,
}

/// A candidate memory entry with its initial retrieval score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankCandidate {
    /// Memory entry
    pub entry: MemoryEntry,
    /// Original retrieval score from triple-stream
    pub original_score: f32,
    /// Individual stream scores (optional)
    pub vector_score: Option<f32>,
    pub bm25_score: Option<f32>,
    pub graph_score: Option<f32>,
}

impl RerankCandidate {
    /// Create a new rerank candidate
    pub fn new(entry: MemoryEntry, original_score: f32) -> Self {
        Self {
            entry,
            original_score,
            vector_score: None,
            bm25_score: None,
            graph_score: None,
        }
    }

    /// Create with individual stream scores
    pub fn with_stream_scores(
        entry: MemoryEntry,
        original_score: f32,
        vector_score: Option<f32>,
        bm25_score: Option<f32>,
        graph_score: Option<f32>,
    ) -> Self {
        Self {
            entry,
            original_score,
            vector_score,
            bm25_score,
            graph_score,
        }
    }
}

/// Result from reranking with cross-encoder scores
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankResult {
    /// Memory entry
    pub entry: MemoryEntry,
    /// Cross-encoder relevance score (0.0 to 1.0)
    pub cross_encoder_score: f32,
    /// Original retrieval score from triple-stream
    pub original_score: f32,
    /// Combined score (configurable blend)
    pub final_score: f32,
    /// Rank after reranking
    pub rank: usize,
}

impl RerankResult {
    /// Create a new rerank result
    pub fn new(
        entry: MemoryEntry,
        cross_encoder_score: f32,
        original_score: f32,
        rank: usize,
    ) -> Self {
        Self {
            final_score: cross_encoder_score, // Default to cross-encoder score
            cross_encoder_score,
            entry,
            original_score,
            rank,
        }
    }

    /// Create with custom final score blend
    pub fn with_blend(
        entry: MemoryEntry,
        cross_encoder_score: f32,
        original_score: f32,
        blend_weight: f32,
        rank: usize,
    ) -> Self {
        // Blend cross-encoder score with original score
        // blend_weight of 1.0 means use only cross-encoder, 0.0 means use only original
        let final_score =
            blend_weight * cross_encoder_score + (1.0 - blend_weight) * original_score;
        Self {
            entry,
            cross_encoder_score,
            original_score,
            final_score,
            rank,
        }
    }
}

/// Score from cross-encoder model
#[derive(Debug, Clone)]
pub struct CrossEncoderScore {
    /// The ID of the memory entry
    pub id: Uuid,
    /// Relevance score from the cross-encoder (typically 0.0 to 1.0 for probability)
    pub score: f32,
}

/// Trait for cross-encoder rerankers
#[async_trait]
pub trait CrossEncoderReranker: Send + Sync {
    /// Rerank a list of candidates given a query
    async fn rerank(
        &self,
        query: &str,
        candidates: Vec<RerankCandidate>,
    ) -> Result<Vec<RerankResult>, SwellError>;

    /// Check if the reranker is available (model loaded, etc.)
    async fn is_available(&self) -> bool;

    /// Get the name of the reranker model
    fn model_name(&self) -> &str;
}

// ============================================================================
// Simple Keyword-Based Reranker (MVP Implementation)
// ============================================================================

/// A simple reranker that uses keyword overlap and length normalization.
/// This is a fallback when ONNX-based BGE reranker is not available.
pub struct SimpleReranker {
    config: CrossEncoderConfig,
}

impl SimpleReranker {
    pub fn new(config: CrossEncoderConfig) -> Self {
        Self { config }
    }

    /// Calculate a simple relevance score based on keyword overlap
    fn score_keywords(&self, query: &str, document: &str) -> f32 {
        let query_terms = self.tokenize(query);
        let doc_terms = self.tokenize(document);

        if query_terms.is_empty() || doc_terms.is_empty() {
            return 0.0;
        }

        let query_set: HashSet<&str> = query_terms.iter().map(|s| s.as_str()).collect();
        let doc_set: HashSet<&str> = doc_terms.iter().map(|s| s.as_str()).collect();

        // Calculate Jaccard similarity
        let intersection: HashSet<_> = query_set.intersection(&doc_set).collect();
        let union: HashSet<_> = query_set.union(&doc_set).collect();

        if union.is_empty() {
            return 0.0;
        }

        intersection.len() as f32 / union.len() as f32
    }

    /// Tokenize text into lowercase words
    fn tokenize(&self, text: &str) -> Vec<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty() && s.len() > 1)
            .map(|s| s.to_string())
            .collect()
    }

    /// Score based on exact phrase matches
    fn score_phrases(&self, query: &str, document: &str) -> f32 {
        let query_lower = query.to_lowercase();
        let doc_lower = document.to_lowercase();

        // Count how many query words appear in consecutive positions
        let query_words: Vec<&str> = query_lower
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .collect();

        if query_words.len() < 2 {
            return 0.0;
        }

        let mut phrase_matches = 0;
        for window_size in 2..=query_words.len().min(4) {
            for i in 0..=(query_words.len() - window_size) {
                let phrase: String = query_words[i..i + window_size].join(" ");
                if doc_lower.contains(&phrase) {
                    phrase_matches += 1;
                }
            }
        }

        // Normalize by number of possible phrases
        // For each window_size, there are (len - window_size + 1) possible phrases
        let mut num_phrases: f32 = 0.0;
        for window_size in 2..=4 {
            if window_size <= query_words.len() {
                num_phrases += (query_words.len() - window_size + 1) as f32;
            }
        }

        if num_phrases <= 0.0 {
            return 0.0;
        }

        (phrase_matches as f32 / num_phrases).min(1.0)
    }

    /// Score based on query term frequency in document
    fn score_term_density(&self, query: &str, document: &str) -> f32 {
        let query_terms = self.tokenize(query);
        let doc_lower = document.to_lowercase();

        if query_terms.is_empty() {
            return 0.0;
        }

        let mut total_tf: f32 = 0.0;
        let mut matched_terms: f32 = 0.0;

        for term in &query_terms {
            let count = doc_lower.matches(&term.to_lowercase()).count() as f32;
            if count > 0.0 {
                matched_terms += 1.0;
                // Log-scaled term frequency to prevent spam
                total_tf += (1.0 + count.ln()).min(5.0);
            }
        }

        // Combine coverage and density
        let coverage = matched_terms / query_terms.len() as f32;
        let avg_density = if matched_terms > 0.0 {
            total_tf / matched_terms
        } else {
            0.0
        };

        // Balance coverage and density
        coverage * 0.6 + (avg_density / 5.0).min(1.0) * 0.4
    }
}

#[async_trait]
impl CrossEncoderReranker for SimpleReranker {
    async fn rerank(
        &self,
        query: &str,
        candidates: Vec<RerankCandidate>,
    ) -> Result<Vec<RerankResult>, SwellError> {
        let max_results = self.config.max_results;

        // Score each candidate
        let mut scored: Vec<RerankResult> = candidates
            .into_iter()
            .map(|candidate| {
                let content = format!("{}\n\n{}", candidate.entry.label, candidate.entry.content);
                let label_score = self.score_keywords(query, &candidate.entry.label);
                let content_score = self.score_keywords(query, &content);
                let phrase_score = self.score_phrases(query, &content);
                let density_score = self.score_term_density(query, &content);

                // Weighted combination of scoring methods
                let cross_encoder_score = label_score * 0.2
                    + content_score * 0.3
                    + phrase_score * 0.3
                    + density_score * 0.2;

                RerankResult::new(
                    candidate.entry,
                    cross_encoder_score,
                    candidate.original_score,
                    0,
                )
            })
            .collect();

        // Sort by cross-encoder score descending
        scored.sort_by(|a, b| {
            b.cross_encoder_score
                .partial_cmp(&a.cross_encoder_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Assign ranks and truncate
        for (i, result) in scored.iter_mut().enumerate() {
            result.rank = i + 1;
        }

        scored.truncate(max_results);

        Ok(scored)
    }

    async fn is_available(&self) -> bool {
        true // Simple reranker is always available
    }

    fn model_name(&self) -> &str {
        "simple-keyword-reranker"
    }
}

// ============================================================================
// Mock Reranker for Testing
// ============================================================================

/// A mock reranker that returns deterministic scores based on rank position.
/// Useful for testing the reranking integration without a real model.
pub struct MockReranker {
    config: CrossEncoderConfig,
}

impl MockReranker {
    pub fn new(config: CrossEncoderConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl CrossEncoderReranker for MockReranker {
    async fn rerank(
        &self,
        _query: &str,
        candidates: Vec<RerankCandidate>,
    ) -> Result<Vec<RerankResult>, SwellError> {
        let max_results = self.config.max_results;

        // Score based on original rank (lower rank = higher score)
        let mut scored: Vec<RerankResult> = candidates
            .into_iter()
            .enumerate()
            .map(|(i, candidate)| {
                // Invert the original score (higher original score = lower position)
                let cross_encoder_score = 1.0 / (i + 1) as f32;
                RerankResult::new(
                    candidate.entry,
                    cross_encoder_score,
                    candidate.original_score,
                    i + 1,
                )
            })
            .collect();

        // For mock, just use the order they came in
        for (i, result) in scored.iter_mut().enumerate() {
            result.rank = i + 1;
        }

        scored.truncate(max_results);

        Ok(scored)
    }

    async fn is_available(&self) -> bool {
        true
    }

    fn model_name(&self) -> &str {
        "mock-reranker"
    }
}

// ============================================================================
// Cross-Encoder Reranking Service
// ============================================================================

/// High-level service for cross-encoder reranking
pub struct CrossEncoderService {
    reranker: Box<dyn CrossEncoderReranker>,
    config: CrossEncoderConfig,
}

impl CrossEncoderService {
    /// Create a new CrossEncoderService with a simple reranker
    pub fn with_simple_reranker(config: CrossEncoderConfig) -> Self {
        Self {
            reranker: Box::new(SimpleReranker::new(config.clone())),
            config,
        }
    }

    /// Create a new CrossEncoderService with a mock reranker
    #[allow(dead_code)]
    pub fn with_mock_reranker(config: CrossEncoderConfig) -> Self {
        Self {
            reranker: Box::new(MockReranker::new(config.clone())),
            config,
        }
    }

    /// Create with a custom reranker
    pub fn with_reranker<R: CrossEncoderReranker + 'static>(
        reranker: R,
        config: CrossEncoderConfig,
    ) -> Self {
        Self {
            reranker: Box::new(reranker),
            config,
        }
    }

    /// Rerank candidates from a retrieval query
    pub async fn rerank(
        &self,
        query: &str,
        candidates: Vec<RerankCandidate>,
    ) -> Result<Vec<RerankResult>, SwellError> {
        if !self.config.enabled {
            // If reranking is disabled, just convert candidates to results
            let results: Vec<RerankResult> = candidates
                .into_iter()
                .enumerate()
                .map(|(i, c)| RerankResult::new(c.entry, c.original_score, c.original_score, i + 1))
                .collect();
            return Ok(results);
        }

        // Limit candidates to max_candidates
        let candidates = if candidates.len() > self.config.max_candidates {
            candidates
                .into_iter()
                .take(self.config.max_candidates)
                .collect()
        } else {
            candidates
        };

        self.reranker.rerank(query, candidates).await
    }

    /// Check if reranking is available
    pub async fn is_available(&self) -> bool {
        self.reranker.is_available().await
    }

    /// Get the model name
    pub fn model_name(&self) -> &str {
        self.reranker.model_name()
    }
}

/// Extension trait to add reranking to TripleStream results
pub trait RerankableCandidates {
    /// Convert TripleStreamResults to RerankCandidates
    fn to_rerank_candidates(self) -> Vec<RerankCandidate>;
}

impl RerankableCandidates for Vec<crate::triple_stream::TripleStreamResult> {
    fn to_rerank_candidates(self) -> Vec<RerankCandidate> {
        self.into_iter()
            .map(|result| {
                RerankCandidate::with_stream_scores(
                    MemoryEntry {
                        id: result.id,
                        block_type: crate::MemoryBlockType::Task, // Will be filled by caller
                        label: String::new(),
                        content: String::new(),
                        embedding: None,
                        created_at: chrono::Utc::now(),
                        updated_at: chrono::Utc::now(),
                        metadata: serde_json::json!({}),
                        repository: String::new(),
                        org: String::new(),
                        workspace: String::new(),
                        language: None,
                        framework: None,
                        environment: None,
                        task_type: None,
                        session_id: None,
                        last_reinforcement: None,
                        is_stale: false,
                        source_episode_id: None,
                        evidence: None,
                        provenance_context: None,
                    },
                    result.score,
                    result.vector_score,
                    result.bm25_score,
                    result.graph_score,
                )
            })
            .collect()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryBlockType;

    fn create_test_candidates() -> Vec<RerankCandidate> {
        vec![
            RerankCandidate::new(
                MemoryEntry {
                    id: Uuid::new_v4(),
                    block_type: MemoryBlockType::Project,
                    label: "Rust error handling".to_string(),
                    content: "This module handles errors in Rust using the Result type."
                        .to_string(),
                    embedding: None,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                    metadata: serde_json::json!({}),
                    repository: "test-repo".to_string(),
                    language: Some("rust".to_string()),
                    task_type: None,
                    org: String::new(),
                    workspace: String::new(),
                    framework: None,
                    environment: None,
                    session_id: None,

                    last_reinforcement: None,
                    is_stale: false,
                    source_episode_id: None,
                    evidence: None,
                    provenance_context: None,
                },
                0.8,
            ),
            RerankCandidate::new(
                MemoryEntry {
                    id: Uuid::new_v4(),
                    block_type: MemoryBlockType::Task,
                    label: "Python tutorial".to_string(),
                    content: "Learn Python programming from scratch.".to_string(),
                    embedding: None,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                    metadata: serde_json::json!({}),
                    repository: "test-repo".to_string(),
                    language: Some("python".to_string()),
                    task_type: None,
                    org: String::new(),
                    workspace: String::new(),
                    framework: None,
                    environment: None,
                    session_id: None,

                    last_reinforcement: None,
                    is_stale: false,
                    source_episode_id: None,
                    evidence: None,
                    provenance_context: None,
                },
                0.7,
            ),
            RerankCandidate::new(
                MemoryEntry {
                    id: Uuid::new_v4(),
                    block_type: MemoryBlockType::Convention,
                    label: "Rust naming conventions".to_string(),
                    content: "Use snake_case for functions and PascalCase for types in Rust."
                        .to_string(),
                    embedding: None,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                    metadata: serde_json::json!({}),
                    repository: "test-repo".to_string(),
                    language: Some("rust".to_string()),
                    task_type: None,
                    org: String::new(),
                    workspace: String::new(),
                    framework: None,
                    environment: None,
                    session_id: None,

                    last_reinforcement: None,
                    is_stale: false,
                    source_episode_id: None,
                    evidence: None,
                    provenance_context: None,
                },
                0.6,
            ),
        ]
    }

    #[tokio::test]
    async fn test_simple_reranker_basic_scoring() {
        let config = CrossEncoderConfig::default();
        let reranker = SimpleReranker::new(config);

        let candidates = create_test_candidates();
        let results = reranker
            .rerank("Rust error handling", candidates)
            .await
            .unwrap();

        assert_eq!(results.len(), 3);
        // Rust-related entries should score higher
        let rust_entries: Vec<_> = results
            .iter()
            .filter(|r| r.entry.language.as_deref() == Some("rust"))
            .collect();
        assert!(rust_entries.len() >= 2);
        // The first result should be Rust-related
        assert_eq!(results[0].entry.language.as_deref(), Some("rust"));
    }

    #[tokio::test]
    async fn test_simple_reranker_phrase_matching() {
        let config = CrossEncoderConfig::default();
        let reranker = SimpleReranker::new(config);

        let candidates = vec![RerankCandidate::new(
            MemoryEntry {
                id: Uuid::new_v4(),
                block_type: MemoryBlockType::Task,
                label: "Error handling patterns".to_string(),
                content: "Error handling patterns in Rust using Result and Option types."
                    .to_string(),
                embedding: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                metadata: serde_json::json!({}),
                repository: "test-repo".to_string(),
                language: Some("rust".to_string()),
                task_type: None,
                org: String::new(),
                workspace: String::new(),
                framework: None,
                environment: None,
                session_id: None,

                last_reinforcement: None,
                is_stale: false,
                source_episode_id: None,
                evidence: None,
                provenance_context: None,
            },
            0.5,
        )];

        let results = reranker
            .rerank("Rust error handling", candidates)
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        // Phrase match should give high score
        assert!(results[0].cross_encoder_score > 0.3);
    }

    #[tokio::test]
    async fn test_simple_reranker_keyword_overlap() {
        let config = CrossEncoderConfig::default();
        let reranker = SimpleReranker::new(config);

        // Test with exact keyword overlap
        let candidates = vec![
            RerankCandidate::new(
                MemoryEntry {
                    id: Uuid::new_v4(),
                    block_type: MemoryBlockType::Project,
                    label: "Testing module".to_string(),
                    content: "Unit testing in Rust with cargo test.".to_string(),
                    embedding: None,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                    metadata: serde_json::json!({}),
                    repository: "test-repo".to_string(),
                    language: Some("rust".to_string()),
                    task_type: None,
                    org: String::new(),
                    workspace: String::new(),
                    framework: None,
                    environment: None,
                    session_id: None,

                    last_reinforcement: None,
                    is_stale: false,
                    source_episode_id: None,
                    evidence: None,
                    provenance_context: None,
                },
                0.5,
            ),
            RerankCandidate::new(
                MemoryEntry {
                    id: Uuid::new_v4(),
                    block_type: MemoryBlockType::Project,
                    label: "Database layer".to_string(),
                    content: "SQLite database integration for storage.".to_string(),
                    embedding: None,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                    metadata: serde_json::json!({}),
                    repository: "test-repo".to_string(),
                    language: Some("rust".to_string()),
                    task_type: None,
                    org: String::new(),
                    workspace: String::new(),
                    framework: None,
                    environment: None,
                    session_id: None,

                    last_reinforcement: None,
                    is_stale: false,
                    source_episode_id: None,
                    evidence: None,
                    provenance_context: None,
                },
                0.5,
            ),
        ];

        let results = reranker.rerank("Rust testing", candidates).await.unwrap();

        // Testing entry should score higher for "Rust testing" query
        assert!(results[0].cross_encoder_score >= results[1].cross_encoder_score);
    }

    #[tokio::test]
    async fn test_simple_reranker_is_available() {
        let config = CrossEncoderConfig::default();
        let reranker = SimpleReranker::new(config);
        assert!(reranker.is_available().await);
    }

    #[tokio::test]
    async fn test_simple_reranker_model_name() {
        let config = CrossEncoderConfig::default();
        let reranker = SimpleReranker::new(config);
        assert_eq!(reranker.model_name(), "simple-keyword-reranker");
    }

    #[tokio::test]
    async fn test_mock_reranker_deterministic() {
        let config = CrossEncoderConfig::default();
        let reranker = MockReranker::new(config);

        let candidates = create_test_candidates();
        let results = reranker.rerank("any query", candidates).await.unwrap();

        // Mock reranker returns results in original order with inverse rank scoring
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].rank, 1);
        assert_eq!(results[1].rank, 2);
        assert_eq!(results[2].rank, 3);
    }

    #[tokio::test]
    async fn test_cross_encoder_service_disabled() {
        let config = CrossEncoderConfig {
            enabled: false,
            ..Default::default()
        };
        let service = CrossEncoderService::with_simple_reranker(config);

        let candidates = create_test_candidates();
        let results = service.rerank("test query", candidates).await.unwrap();

        // When disabled, should return candidates with original scores
        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn test_cross_encoder_service_max_candidates() {
        let config = CrossEncoderConfig {
            max_candidates: 2,
            ..Default::default()
        };
        let service = CrossEncoderService::with_simple_reranker(config);

        let candidates = create_test_candidates();
        let results = service.rerank("test query", candidates).await.unwrap();

        // Should be limited to max_candidates
        assert!(results.len() <= 2);
    }

    #[tokio::test]
    async fn test_cross_encoder_service_max_results() {
        let config = CrossEncoderConfig {
            max_results: 1,
            ..Default::default()
        };
        let service = CrossEncoderService::with_simple_reranker(config);

        let candidates = create_test_candidates();
        let results = service.rerank("test query", candidates).await.unwrap();

        // Should be limited to max_results
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_cross_encoder_service_availability() {
        let config = CrossEncoderConfig::default();
        let service = CrossEncoderService::with_simple_reranker(config);
        assert!(service.is_available().await);
    }

    #[test]
    fn test_cross_encoder_config_default() {
        let config = CrossEncoderConfig::default();
        assert!(config.enabled);
        assert_eq!(config.max_candidates, 50);
        assert_eq!(config.max_results, 10);
        assert_eq!(config.max_doc_length, 512);
        assert_eq!(config.batch_size, 8);
    }

    #[test]
    fn test_reranker_model_type_default() {
        assert_eq!(RerankerModelType::default(), RerankerModelType::Simple);
    }

    #[test]
    fn test_rerank_result_creation() {
        let entry = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Task,
            label: "test".to_string(),
            content: "test content".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "test".to_string(),
            language: None,
            task_type: None,
            org: String::new(),
            workspace: String::new(),
            framework: None,
            environment: None,
            session_id: None,

            last_reinforcement: None,
            is_stale: false,
            source_episode_id: None,
            evidence: None,
            provenance_context: None,
        };

        let result = RerankResult::new(entry.clone(), 0.9, 0.7, 1);

        assert_eq!(result.cross_encoder_score, 0.9);
        assert_eq!(result.original_score, 0.7);
        assert_eq!(result.final_score, 0.9); // Default is cross-encoder score
        assert_eq!(result.rank, 1);
    }

    #[test]
    fn test_rerank_result_with_blend() {
        let entry = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Task,
            label: "test".to_string(),
            content: "test content".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "test".to_string(),
            language: None,
            task_type: None,
            org: String::new(),
            workspace: String::new(),
            framework: None,
            environment: None,
            session_id: None,

            last_reinforcement: None,
            is_stale: false,
            source_episode_id: None,
            evidence: None,
            provenance_context: None,
        };

        // 50-50 blend
        let result = RerankResult::with_blend(entry.clone(), 1.0, 0.0, 0.5, 1);

        assert_eq!(result.cross_encoder_score, 1.0);
        assert_eq!(result.original_score, 0.0);
        assert_eq!(result.final_score, 0.5); // 0.5 * 1.0 + 0.5 * 0.0 = 0.5

        // 100% cross-encoder
        let result = RerankResult::with_blend(entry.clone(), 0.8, 0.5, 1.0, 1);
        assert_eq!(result.final_score, 0.8);

        // 100% original
        let result = RerankResult::with_blend(entry.clone(), 0.8, 0.9, 0.0, 1);
        assert_eq!(result.final_score, 0.9);
    }

    #[test]
    fn test_rerank_candidate_creation() {
        let entry = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "test".to_string(),
            content: "test content".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "test".to_string(),
            language: None,
            task_type: None,
            org: String::new(),
            workspace: String::new(),
            framework: None,
            environment: None,
            session_id: None,

            last_reinforcement: None,
            is_stale: false,
            source_episode_id: None,
            evidence: None,
            provenance_context: None,
        };

        let candidate = RerankCandidate::new(entry.clone(), 0.8);

        assert_eq!(candidate.entry.id, entry.id);
        assert_eq!(candidate.original_score, 0.8);
        assert!(candidate.vector_score.is_none());

        let candidate_with_scores = RerankCandidate::with_stream_scores(
            entry.clone(),
            0.9,
            Some(0.8),
            Some(0.7),
            Some(0.6),
        );

        assert_eq!(candidate_with_scores.vector_score, Some(0.8));
        assert_eq!(candidate_with_scores.bm25_score, Some(0.7));
        assert_eq!(candidate_with_scores.graph_score, Some(0.6));
    }

    #[tokio::test]
    async fn test_simple_reranker_empty_query() {
        let config = CrossEncoderConfig::default();
        let reranker = SimpleReranker::new(config);

        let candidates = create_test_candidates();
        let results = reranker.rerank("", candidates).await.unwrap();

        // Should still return results (empty query scores 0, all equal)
        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn test_simple_reranker_empty_candidates() {
        let config = CrossEncoderConfig::default();
        let reranker = SimpleReranker::new(config);

        let results = reranker.rerank("test query", vec![]).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_simple_reranker_single_candidate() {
        let config = CrossEncoderConfig::default();
        let reranker = SimpleReranker::new(config);

        let candidates = vec![RerankCandidate::new(
            MemoryEntry {
                id: Uuid::new_v4(),
                block_type: MemoryBlockType::Task,
                label: "Single test".to_string(),
                content: "Single test content".to_string(),
                embedding: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                metadata: serde_json::json!({}),
                repository: "test-repo".to_string(),
                language: None,
                task_type: None,
                org: String::new(),
                workspace: String::new(),
                framework: None,
                environment: None,
                session_id: None,

                last_reinforcement: None,
                is_stale: false,
                source_episode_id: None,
                evidence: None,
                provenance_context: None,
            },
            0.5,
        )];

        let results = reranker.rerank("test", candidates).await.unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].rank, 1);
    }

    #[test]
    fn test_tokenize() {
        let config = CrossEncoderConfig::default();
        let reranker = SimpleReranker::new(config);

        let tokens = reranker.tokenize("Hello World! This is a Test.");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"this".to_string()));
        assert!(tokens.contains(&"test".to_string()));
        // "is" has length 2, so it passes the filter (> 1 char)
        assert!(tokens.contains(&"is".to_string()));
        // "a" is filtered out (single character)
        assert!(!tokens.contains(&"a".to_string()));
    }

    #[test]
    fn test_score_keywords_identical() {
        let config = CrossEncoderConfig::default();
        let reranker = SimpleReranker::new(config);

        let score = reranker.score_keywords("rust error handling", "rust error handling");
        assert!(
            (score - 1.0).abs() < 0.001,
            "Identical texts should have score 1.0"
        );
    }

    #[test]
    fn test_score_keywords_partial_overlap() {
        let config = CrossEncoderConfig::default();
        let reranker = SimpleReranker::new(config);

        let score = reranker.score_keywords("rust error handling", "rust programming");
        assert!(
            score > 0.0 && score < 1.0,
            "Partial overlap should have score between 0 and 1"
        );
    }

    #[test]
    fn test_score_keywords_no_overlap() {
        let config = CrossEncoderConfig::default();
        let reranker = SimpleReranker::new(config);

        let score = reranker.score_keywords("python tutorial", "rust database");
        assert!(score < 0.1, "No overlap should have near-zero score");
    }

    #[test]
    fn test_score_phrases() {
        let config = CrossEncoderConfig::default();
        let reranker = SimpleReranker::new(config);

        // Exact phrase match
        let score = reranker.score_phrases(
            "error handling in rust",
            "Learn about error handling in rust",
        );
        assert!(score > 0.0, "Should detect phrase match");

        // No phrase match
        let score = reranker.score_phrases("rust python go", "JavaScript TypeScript");
        assert!(score == 0.0, "Should have zero score for no phrase match");
    }

    #[test]
    fn test_score_term_density() {
        let config = CrossEncoderConfig::default();
        let reranker = SimpleReranker::new(config);

        // High density
        let score = reranker.score_term_density(
            "rust error handling",
            "Rust error handling is important. Error handling in Rust uses Result types.",
        );
        assert!(score > 0.5, "High term density should score well");

        // Low density
        let score = reranker.score_term_density("rust error handling", "Python is a language.");
        assert!(score < 0.5, "Low term density should score lower");
    }
}
