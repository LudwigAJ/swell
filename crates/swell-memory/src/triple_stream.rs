// triple_stream.rs - Hybrid retrieval with Vector + BM25 + Graph traversal and Reciprocal Rank Fusion
//
// This module provides hybrid memory retrieval using three complementary search streams:
// 1. Dense Vector Search - Semantic similarity using embeddings
// 2. BM25 Keyword Search - Traditional keyword-based retrieval
// 3. Graph Traversal - Relationship-based retrieval via semantic graph
//
// The streams are combined using Reciprocal Rank Fusion (RRF) to produce a unified ranking.
// For improved relevance, a cross-encoder reranker can optionally re-rank the top candidates.

use crate::cross_encoder_rerank::{
    CrossEncoderConfig, CrossEncoderService, RerankCandidate, RerankResult, RerankerModelType,
};
use crate::recall::{RecallQuery, RecallService};
use crate::semantic::{
    SemanticRelationQuery, SemanticRelationType, SemanticStore, SqliteSemanticStore,
};
use crate::{MemorySearchResult, SqliteMemoryStore, SwellError};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::{HashMap, HashSet};
use swell_core::MemoryStore;
use uuid::Uuid;

/// Configuration for the triple-stream retrieval
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TripleStreamConfig {
    /// Weight for vector search stream (default: 1.0)
    pub vector_weight: f32,
    /// Weight for BM25 stream (default: 1.0)
    pub bm25_weight: f32,
    /// Weight for graph traversal stream (default: 1.0)
    pub graph_weight: f32,
    /// RRF k parameter - controls how much the ranking matters (default: 60)
    /// Higher values make the ranking matter more
    pub rrf_k: u32,
    /// Maximum results to return from each stream before fusion
    pub max_stream_results: usize,
    /// Whether to use vector search (requires embeddings)
    pub enable_vector: bool,
    /// Whether to use BM25 search
    pub enable_bm25: bool,
    /// Whether to use graph traversal
    pub enable_graph: bool,
    /// Cross-encoder reranking configuration (optional)
    pub reranker: Option<CrossEncoderConfig>,
}

impl Default for TripleStreamConfig {
    fn default() -> Self {
        Self {
            vector_weight: 1.0,
            bm25_weight: 1.0,
            graph_weight: 1.0,
            rrf_k: 60,
            max_stream_results: 100,
            enable_vector: true,
            enable_bm25: true,
            enable_graph: true,
            reranker: Some(CrossEncoderConfig::default()),
        }
    }
}

/// Query for triple-stream retrieval
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TripleStreamQuery {
    /// Text query for vector and BM25 search
    pub query_text: String,
    /// Keywords for BM25 (derived from query_text if not provided)
    pub keywords: Option<Vec<String>>,
    /// Repository scope for memory isolation
    pub repository: String,
    /// Starting entity IDs for graph traversal (optional)
    pub graph_seed_ids: Option<Vec<Uuid>>,
    /// Relation types to traverse in graph (default: all types)
    pub graph_relation_types: Option<Vec<SemanticRelationType>>,
    /// Graph traversal depth (default: 2)
    pub graph_depth: usize,
    /// Maximum number of final results
    pub limit: usize,
    /// Offset for pagination
    pub offset: usize,
    /// BM25 parameters (optional)
    pub bm25_params: Option<crate::recall::Bm25Params>,
    /// Configuration for the triple-stream retrieval
    pub config: TripleStreamConfig,
}

impl Default for TripleStreamQuery {
    fn default() -> Self {
        Self {
            query_text: String::new(),
            keywords: None,
            repository: String::new(),
            graph_seed_ids: None,
            graph_relation_types: None,
            graph_depth: 2,
            limit: 10,
            offset: 0,
            bm25_params: None,
            config: TripleStreamConfig::default(),
        }
    }
}

/// Result from triple-stream retrieval
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TripleStreamResult {
    /// Memory entry ID
    pub id: Uuid,
    /// Combined RRF score
    pub score: f32,
    /// Individual stream scores
    pub vector_score: Option<f32>,
    pub bm25_score: Option<f32>,
    pub graph_score: Option<f32>,
    /// Vector rank in the fusion
    pub vector_rank: Option<u32>,
    /// BM25 rank in the fusion
    pub bm25_rank: Option<u32>,
    /// Graph rank in the fusion
    pub graph_rank: Option<u32>,
}

/// Stream result for ranking fusion
#[derive(Debug, Clone)]
pub struct RankedItem {
    pub id: Uuid,
    pub score: f32,
    pub rank: u32,
}

// ============================================================================
// Vector Search Component
// ============================================================================

impl SqliteMemoryStore {
    /// Search memories using dense vector similarity
    /// Returns top results with cosine similarity scores
    pub async fn vector_search(
        &self,
        query_embedding: &[f32],
        repository: &str,
        limit: usize,
    ) -> Result<Vec<RankedItem>, SwellError> {
        // Fetch all entries with embeddings in the repository
        let rows = sqlx::query(
            r#"
            SELECT id, embedding FROM memory_entries 
            WHERE repository = ? AND embedding IS NOT NULL AND is_stale = 0
            "#,
        )
        .bind(repository)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut results: Vec<(Uuid, f32)> = Vec::new();

        for row in rows {
            let id_str: String = row.get("id");
            if let Ok(id) = Uuid::parse_str(&id_str) {
                let embedding_bytes: Option<Vec<u8>> = row.get("embedding");
                if let Some(bytes) = embedding_bytes {
                    let embedding = Self::bytes_to_embedding(&bytes);
                    let similarity = Self::cosine_similarity(query_embedding, &embedding);
                    results.push((id, similarity));
                }
            }
        }

        // Sort by similarity descending
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Take top results and create ranked items
        let ranked: Vec<RankedItem> = results
            .into_iter()
            .take(limit)
            .enumerate()
            .map(|(i, (id, score))| RankedItem {
                id,
                score,
                rank: (i + 1) as u32,
            })
            .collect();

        Ok(ranked)
    }

    /// Calculate cosine similarity between two embeddings
    /// Returns a value between 0 (orthogonal) and 1 (identical)
    fn cosine_similarity(embedding1: &[f32], embedding2: &[f32]) -> f32 {
        if embedding1.len() != embedding2.len() {
            return 0.0;
        }

        let dot_product: f32 = embedding1
            .iter()
            .zip(embedding2.iter())
            .map(|(a, b)| a * b)
            .sum();

        let norm1: f32 = embedding1.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm2: f32 = embedding2.iter().map(|x| x * x).sum::<f32>().sqrt();

        if norm1 == 0.0 || norm2 == 0.0 {
            return 0.0;
        }

        // Cosine similarity = dot / (norm1 * norm2)
        // Clamp to handle floating point errors
        (dot_product / (norm1 * norm2)).clamp(-1.0, 1.0)
    }
}

// ============================================================================
// Graph Traversal Component
// ============================================================================

/// Graph traversal for relationship-based retrieval
pub struct GraphTraversal<'a> {
    semantic_store: &'a SqliteSemanticStore,
}

impl<'a> GraphTraversal<'a> {
    pub fn new(semantic_store: &'a SqliteSemanticStore) -> Self {
        Self { semantic_store }
    }

    /// Traverse the semantic graph starting from seed entities
    /// Returns entities discovered through relationship traversal
    pub async fn traverse(
        &self,
        seed_ids: &[Uuid],
        max_depth: usize,
        relation_types: Option<&[SemanticRelationType]>,
        limit: usize,
    ) -> Result<Vec<RankedItem>, SwellError> {
        let mut visited: HashSet<Uuid> = seed_ids.iter().cloned().collect();
        let mut current_frontier: HashSet<Uuid> = seed_ids.iter().cloned().collect();
        let mut all_entities: Vec<(Uuid, usize)> = Vec::new(); // (entity_id, depth)

        // BFS traversal
        for depth in 1..=max_depth {
            if current_frontier.is_empty() {
                break;
            }

            let mut next_frontier: HashSet<Uuid> = HashSet::new();
            let mut frontier_with_depth: Vec<(Uuid, usize)> = Vec::new();

            for entity_id in &current_frontier {
                // Get all relations (both incoming and outgoing)
                let relations = self.semantic_store.get_entity_relations(*entity_id).await?;

                for relation in relations {
                    // Filter by relation types if specified
                    if let Some(types) = relation_types {
                        if !types.contains(&relation.relation_type) {
                            continue;
                        }
                    }

                    // Add target if not visited
                    if !visited.contains(&relation.target_id) {
                        visited.insert(relation.target_id);
                        next_frontier.insert(relation.target_id);
                        frontier_with_depth.push((relation.target_id, depth));
                    }
                    // Add source if not visited
                    if !visited.contains(&relation.source_id) {
                        visited.insert(relation.source_id);
                        next_frontier.insert(relation.source_id);
                        frontier_with_depth.push((relation.source_id, depth));
                    }
                }
            }

            all_entities.extend(frontier_with_depth);
            current_frontier = next_frontier;
        }

        // Score entities by their proximity to seed (closer = higher score)
        // Also consider connectivity (more connections = higher score)
        let connectivity = self.count_connections(&visited).await?;

        let mut scored: Vec<(Uuid, f32)> = all_entities
            .into_iter()
            .map(|(id, depth)| {
                // Score = connectivity * (1 / depth) - closer and more connected = higher score
                let conn_score = connectivity.get(&id).copied().unwrap_or(1) as f32;
                let depth_score = 1.0 / (depth as f32).max(1.0);
                (id, conn_score * depth_score)
            })
            .collect();

        // Deduplicate and take top results
        let mut seen: HashSet<Uuid> = HashSet::new();
        scored.retain(|(id, _)| seen.insert(*id));

        // Sort by score descending
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let ranked: Vec<RankedItem> = scored
            .into_iter()
            .take(limit)
            .enumerate()
            .map(|(i, (id, score))| RankedItem {
                id,
                score,
                rank: (i + 1) as u32,
            })
            .collect();

        Ok(ranked)
    }

    /// Count number of connections for each entity
    async fn count_connections(
        &self,
        entity_ids: &HashSet<Uuid>,
    ) -> Result<HashMap<Uuid, usize>, SwellError> {
        let mut counts: HashMap<Uuid, usize> = HashMap::new();

        for id in entity_ids {
            let relations = self.semantic_store.get_entity_relations(*id).await?;
            counts.insert(*id, relations.len());
        }

        Ok(counts)
    }

    /// Find entities related to seed entities via specific relation types
    pub async fn find_related(
        &self,
        seed_ids: &[Uuid],
        relation_types: &[SemanticRelationType],
        limit: usize,
    ) -> Result<Vec<RankedItem>, SwellError> {
        let mut all_related: HashMap<Uuid, usize> = HashMap::new(); // entity_id -> number_of_connections

        for seed_id in seed_ids {
            let query = SemanticRelationQuery::new()
                .with_relation_types(relation_types.to_vec())
                .with_source(*seed_id);

            let outgoing = self.semantic_store.query_relations(query).await?;

            for relation in outgoing {
                *all_related.entry(relation.target_id).or_insert(0) += 1;
            }

            // Also check incoming relations
            let query2 = SemanticRelationQuery::new()
                .with_relation_types(relation_types.to_vec())
                .with_target(*seed_id);

            let incoming = self.semantic_store.query_relations(query2).await?;

            for relation in incoming {
                *all_related.entry(relation.source_id).or_insert(0) += 1;
            }
        }

        // Sort by number of connections
        let mut sorted: Vec<(Uuid, usize)> = all_related.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));

        let ranked: Vec<RankedItem> = sorted
            .into_iter()
            .take(limit)
            .enumerate()
            .map(|(i, (id, connections))| RankedItem {
                id,
                score: connections as f32,
                rank: (i + 1) as u32,
            })
            .collect();

        Ok(ranked)
    }
}

// ============================================================================
// Reciprocal Rank Fusion
// ============================================================================

/// Reciprocal Rank Fusion implementation
/// Combines rankings from multiple streams using the RRF formula:
/// RRFscore(d) = Σ 1/(k + rank(d))
/// where k is a constant (typically 60) and rank(d) is the rank in stream i
pub struct ReciprocalRankFusion {
    k: u32,
}

impl ReciprocalRankFusion {
    pub fn new(k: u32) -> Self {
        Self { k }
    }

    /// Default RRF with k=60
    pub fn default_rrf() -> Self {
        Self::new(60)
    }

    /// Fuse multiple ranked lists into a single ranking
    /// Each input is a mapping from ID to rank (1-based rank, 0 means not ranked)
    pub fn fuse(
        &self,
        rankings: Vec<HashMap<Uuid, u32>>,
        weights: Option<&[f32]>,
    ) -> Vec<(Uuid, f32)> {
        let mut scores: HashMap<Uuid, f32> = HashMap::new();
        let weights = weights.unwrap_or(&[1.0; 100]); // Dummy large array, will use actual length

        for (stream_idx, ranking) in rankings.iter().enumerate() {
            let weight = weights.get(stream_idx).copied().unwrap_or(1.0);

            for (id, &rank) in ranking {
                if rank > 0 {
                    // RRF formula: weight / (k + rank)
                    let rrf_score = weight / (self.k as f32 + rank as f32);
                    *scores.entry(*id).or_insert(0.0) += rrf_score;
                }
            }
        }

        // Sort by RRF score descending
        let mut sorted: Vec<(Uuid, f32)> = scores.into_iter().collect();
        sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        sorted
    }

    /// Fuse ranked items from different streams
    pub fn fuse_ranked(
        &self,
        stream_results: Vec<Vec<RankedItem>>,
        weights: &[f32],
    ) -> Vec<(Uuid, f32)> {
        let rankings: Vec<HashMap<Uuid, u32>> = stream_results
            .iter()
            .map(|items| items.iter().map(|item| (item.id, item.rank)).collect())
            .collect();

        self.fuse(rankings, Some(weights))
    }
}

// ============================================================================
// Triple Stream Service
// ============================================================================

/// High-level service for triple-stream retrieval
pub struct TripleStreamService {
    memory_store: SqliteMemoryStore,
    semantic_store: SqliteSemanticStore,
    recall_service: RecallService,
    #[allow(dead_code)]
    config: TripleStreamConfig,
}

impl TripleStreamService {
    pub fn new(
        memory_store: SqliteMemoryStore,
        semantic_store: SqliteSemanticStore,
        recall_service: RecallService,
    ) -> Self {
        Self {
            memory_store,
            semantic_store,
            recall_service,
            config: TripleStreamConfig::default(),
        }
    }

    pub fn with_config(
        memory_store: SqliteMemoryStore,
        semantic_store: SqliteSemanticStore,
        recall_service: RecallService,
        config: TripleStreamConfig,
    ) -> Self {
        Self {
            memory_store,
            semantic_store,
            recall_service,
            config,
        }
    }

    /// Perform triple-stream retrieval and fuse results
    pub async fn search(
        &self,
        query: TripleStreamQuery,
    ) -> Result<Vec<TripleStreamResult>, SwellError> {
        let config = &query.config;

        // Track ranks for each stream
        let mut vector_ranks: HashMap<Uuid, u32> = HashMap::new();
        let mut bm25_ranks: HashMap<Uuid, u32> = HashMap::new();
        let mut graph_ranks: HashMap<Uuid, u32> = HashMap::new();

        let mut stream_results: Vec<Vec<RankedItem>> = Vec::new();
        let mut weights: Vec<f32> = Vec::new();

        // 1. Vector Search Stream
        if config.enable_vector {
            // For vector search, we need a query embedding
            // In a real implementation, this would use an embedding model
            // For now, we use a placeholder that returns results based on content match
            let vector_results = self.vector_search_stream(&query).await?;
            if !vector_results.is_empty() {
                for item in &vector_results {
                    vector_ranks.insert(item.id, item.rank);
                }
                stream_results.push(vector_results);
                weights.push(config.vector_weight);
            }
        }

        // 2. BM25 Stream
        if config.enable_bm25 {
            let bm25_results = self.bm25_search_stream(&query).await?;
            if !bm25_results.is_empty() {
                for item in &bm25_results {
                    bm25_ranks.insert(item.id, item.rank);
                }
                stream_results.push(bm25_results);
                weights.push(config.bm25_weight);
            }
        }

        // 3. Graph Traversal Stream
        if config.enable_graph && query.graph_seed_ids.is_some() {
            let graph_results = self.graph_traversal_stream(&query).await?;
            if !graph_results.is_empty() {
                for item in &graph_results {
                    graph_ranks.insert(item.id, item.rank);
                }
                stream_results.push(graph_results);
                weights.push(config.graph_weight);
            }
        }

        // If no streams enabled or no results, return empty
        if stream_results.is_empty() {
            return Ok(Vec::new());
        }

        // Fuse rankings using RRF
        let rrf = ReciprocalRankFusion::new(config.rrf_k);
        let fused = rrf.fuse_ranked(stream_results, &weights);

        // Build results with individual scores
        let mut results: Vec<TripleStreamResult> = Vec::new();
        for (id, score) in fused {
            results.push(TripleStreamResult {
                id,
                score,
                vector_score: vector_ranks
                    .get(&id)
                    .map(|r| 1.0 / (config.rrf_k as f32 + *r as f32)),
                bm25_score: bm25_ranks
                    .get(&id)
                    .map(|r| 1.0 / (config.rrf_k as f32 + *r as f32)),
                graph_score: graph_ranks
                    .get(&id)
                    .map(|r| 1.0 / (config.rrf_k as f32 + *r as f32)),
                vector_rank: vector_ranks.get(&id).copied(),
                bm25_rank: bm25_ranks.get(&id).copied(),
                graph_rank: graph_ranks.get(&id).copied(),
            });
        }

        // Apply pagination
        let total = results.len();
        results.truncate(query.limit);
        if query.offset > 0 && query.offset < total {
            results = results.into_iter().skip(query.offset).collect();
        }

        Ok(results)
    }

    /// Perform triple-stream retrieval with optional cross-encoder reranking.
    ///
    /// This method first performs standard triple-stream retrieval using RRF fusion,
    /// then optionally re-ranks the top candidates using a cross-encoder model.
    ///
    /// The cross-encoder reranker scores query-document pairs jointly, providing
    /// more accurate relevance scoring than bi-encoders used in the initial retrieval.
    ///
    /// # Arguments
    /// * `query` - The search query
    /// * `reranker_config` - Optional cross-encoder configuration. If None, reranking is skipped.
    ///
    /// # Returns
    /// * `Vec<TripleStreamResult>` - Re-ranked results with cross-encoder scores
    pub async fn search_with_reranking(
        &self,
        query: TripleStreamQuery,
        reranker_config: Option<CrossEncoderConfig>,
    ) -> Result<Vec<TripleStreamResult>, SwellError> {
        // First, perform standard triple-stream search to get initial rankings
        let initial_results = self.search(query.clone()).await?;

        if initial_results.is_empty() {
            return Ok(Vec::new());
        }

        // If no reranker config, return initial results
        let reranker_config = match reranker_config {
            Some(config) if config.enabled => config,
            _ => return Ok(initial_results),
        };

        // Get full memory entries for the candidates
        let candidates = self.build_rerank_candidates(&initial_results).await?;

        if candidates.is_empty() {
            return Ok(initial_results);
        }

        // Create cross-encoder service
        let reranker_type = reranker_config.model_type;
        let cross_encoder_service = match reranker_type {
            RerankerModelType::Simple | RerankerModelType::Bge => {
                CrossEncoderService::with_simple_reranker(reranker_config)
            }
            RerankerModelType::Mock => CrossEncoderService::with_mock_reranker(reranker_config),
        };

        // Perform reranking
        let reranked = cross_encoder_service
            .rerank(&query.query_text, candidates)
            .await?;

        // Convert reranked results back to TripleStreamResult format
        let results = self.reranked_to_triple_stream_results(reranked, initial_results);

        Ok(results)
    }

    /// Build rerank candidates from triple-stream results
    async fn build_rerank_candidates(
        &self,
        results: &[TripleStreamResult],
    ) -> Result<Vec<RerankCandidate>, SwellError> {
        let mut candidates = Vec::new();

        for result in results {
            // Fetch the full memory entry
            if let Some(entry) = self.memory_store.get(result.id).await? {
                candidates.push(RerankCandidate::with_stream_scores(
                    entry,
                    result.score,
                    result.vector_score,
                    result.bm25_score,
                    result.graph_score,
                ));
            }
        }

        Ok(candidates)
    }

    /// Convert reranked results back to TripleStreamResult format
    fn reranked_to_triple_stream_results(
        &self,
        reranked: Vec<RerankResult>,
        _initial_results: Vec<TripleStreamResult>,
    ) -> Vec<TripleStreamResult> {
        reranked
            .into_iter()
            .map(|result| {
                TripleStreamResult {
                    id: result.entry.id,
                    score: result.final_score,
                    vector_score: None, // Not preserved through reranking
                    bm25_score: None,
                    graph_score: None,
                    vector_rank: None,
                    bm25_rank: None,
                    graph_rank: None,
                }
            })
            .collect()
    }

    /// Vector search stream
    async fn vector_search_stream(
        &self,
        query: &TripleStreamQuery,
    ) -> Result<Vec<RankedItem>, SwellError> {
        // In a real implementation, we would generate an embedding for the query
        // For now, we search using content matching and return mock vector scores
        // This is a placeholder that simulates vector similarity

        // Get all entries with embeddings and match by content
        let entries: Vec<MemorySearchResult> = self
            .memory_store
            .search(crate::MemoryQuery {
                query_text: Some(query.query_text.clone()),
                block_types: None,
                labels: None,
                limit: query.config.max_stream_results,
                offset: 0,
                repository: query.repository.clone(),
                org: String::new(),
                workspace: String::new(),
                language: None,
                framework: None,
                environment: None,
                task_type: None,
                session_id: None,
                source_episode_id: None,
                cross_scope_override: false,
            })
            .await?;

        // Score based on label match vs content match
        let ranked: Vec<RankedItem> = entries
            .into_iter()
            .enumerate()
            .map(|(i, result)| {
                let vector_score = if result
                    .entry
                    .label
                    .to_lowercase()
                    .contains(&query.query_text.to_lowercase())
                {
                    0.9 - (i as f32 * 0.01)
                } else {
                    0.7 - (i as f32 * 0.01)
                };
                RankedItem {
                    id: result.entry.id,
                    score: vector_score,
                    rank: (i + 1) as u32,
                }
            })
            .collect();

        Ok(ranked)
    }

    /// BM25 search stream
    async fn bm25_search_stream(
        &self,
        query: &TripleStreamQuery,
    ) -> Result<Vec<RankedItem>, SwellError> {
        let keywords = if let Some(ref kw) = query.keywords {
            kw.clone()
        } else {
            tokenize_query(&query.query_text)
        };

        let recall_query = RecallQuery {
            keywords,
            session_id: None,
            task_id: None,
            agent_role: None,
            action: None,
            start_time: None,
            end_time: None,
            limit: query.config.max_stream_results,
            offset: 0,
            bm25_params: query.bm25_params,
        };

        let recall_results = self.recall_service.search(recall_query).await?;

        // Convert RecallResults to RankedItems based on memory entry content matching
        // Since recall operates on conversation logs, we need to map back to memory entries
        // For now, we use the BM25 scores directly
        let ranked: Vec<RankedItem> = recall_results
            .iter()
            .enumerate()
            .map(|(i, result)| {
                // Generate a consistent ID from the content for fusion
                // In real implementation, recall results would link to memory entries
                let id = Uuid::new_v4(); // Placeholder
                RankedItem {
                    id,
                    score: result.score,
                    rank: (i + 1) as u32,
                }
            })
            .collect();

        Ok(ranked)
    }

    /// Graph traversal stream
    async fn graph_traversal_stream(
        &self,
        query: &TripleStreamQuery,
    ) -> Result<Vec<RankedItem>, SwellError> {
        let seed_ids = match &query.graph_seed_ids {
            Some(ids) if !ids.is_empty() => ids,
            _ => return Ok(Vec::new()),
        };

        let traversal = GraphTraversal::new(&self.semantic_store);

        let ranked = if let Some(ref types) = query.graph_relation_types {
            traversal
                .find_related(seed_ids, types, query.config.max_stream_results)
                .await?
        } else {
            traversal
                .traverse(
                    seed_ids,
                    query.graph_depth,
                    None,
                    query.config.max_stream_results,
                )
                .await?
        };

        Ok(ranked)
    }
}

/// Tokenize query text into keywords
fn tokenize_query(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty() && s.len() > 2)
        .map(|s| s.to_string())
        .collect()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let embedding1 = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let embedding2 = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let similarity = SqliteMemoryStore::cosine_similarity(&embedding1, &embedding2);
        assert!(
            similarity > 0.99,
            "Identical embeddings should have similarity near 1"
        );
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let embedding1 = vec![1.0, 0.0, 0.0];
        let embedding2 = vec![0.0, 1.0, 0.0];
        let similarity = SqliteMemoryStore::cosine_similarity(&embedding1, &embedding2);
        assert!(
            similarity < 0.01,
            "Orthogonal embeddings should have similarity near 0"
        );
    }

    #[test]
    fn test_cosine_similarity_different_lengths() {
        let embedding1 = vec![0.1, 0.2, 0.3];
        let embedding2 = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let similarity = SqliteMemoryStore::cosine_similarity(&embedding1, &embedding2);
        assert_eq!(
            similarity, 0.0,
            "Different length embeddings should have similarity 0"
        );
    }

    #[test]
    fn test_rrf_fuse_empty() {
        let rrf = ReciprocalRankFusion::default_rrf();
        let rankings: Vec<HashMap<Uuid, u32>> = Vec::new();
        let fused = rrf.fuse(rankings, None);
        assert!(fused.is_empty());
    }

    #[test]
    fn test_rrf_fuse_single_ranking() {
        let rrf = ReciprocalRankFusion::new(60);
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        let rankings = vec![HashMap::from([(id1, 1), (id2, 2)])];

        let fused = rrf.fuse(rankings, None);

        assert_eq!(fused.len(), 2);
        assert_eq!(fused[0].0, id1); // id1 should be first (rank 1)
        assert_eq!(fused[1].0, id2); // id2 should be second (rank 2)
    }

    #[test]
    fn test_rrf_fuse_multiple_rankings() {
        let rrf = ReciprocalRankFusion::new(60);
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        // Stream 1: id1=1, id2=2, id3=3
        // Stream 2: id2=1, id1=2, id3=3
        let rankings = vec![
            HashMap::from([(id1, 1), (id2, 2), (id3, 3)]),
            HashMap::from([(id2, 1), (id1, 2), (id3, 3)]),
        ];

        let fused = rrf.fuse(rankings, None);

        // id1 appears at rank 1 and 2: score = 1/(60+1) + 1/(60+2)
        // id2 appears at rank 2 and 1: score = 1/(60+2) + 1/(60+1)
        // id3 appears at rank 3 and 3: score = 1/(60+3) + 1/(60+3)
        // id1 and id2 should have similar scores
        assert_eq!(fused.len(), 3);
    }

    #[test]
    fn test_rrf_fuse_with_weights() {
        let rrf = ReciprocalRankFusion::new(60);
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        let rankings = vec![
            HashMap::from([(id1, 1), (id2, 2)]),
            HashMap::from([(id2, 1), (id1, 2)]),
        ];

        let weights = vec![2.0, 1.0];

        let fused = rrf.fuse(rankings, Some(&weights));

        // With weights, id1 should rank higher (higher weight in first stream where it's rank 1)
        assert_eq!(fused[0].0, id1);
    }

    #[test]
    fn test_rrf_score_calculation() {
        let rrf = ReciprocalRankFusion::new(60);
        let id = Uuid::new_v4();

        // When id is at rank 1 with weight 1.0:
        // score = 1.0 / (60 + 1) = 1/61 ≈ 0.01639
        let rankings = vec![HashMap::from([(id, 1)])];
        let fused = rrf.fuse(rankings, None);

        assert!((fused[0].1 - 1.0 / 61.0).abs() < 0.001);
    }

    #[test]
    fn test_tokenize_query() {
        let tokens = tokenize_query("Hello World! This is a Test.");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"this".to_string())); // Note: "this" is 4 chars, included
        assert!(tokens.contains(&"test".to_string()));
        // "is" and "a" are filtered out (too short)
        assert!(!tokens.contains(&"is".to_string()));
        assert!(!tokens.contains(&"a".to_string()));
    }

    #[test]
    fn test_triple_stream_config_default() {
        let config = TripleStreamConfig::default();
        assert_eq!(config.vector_weight, 1.0);
        assert_eq!(config.bm25_weight, 1.0);
        assert_eq!(config.graph_weight, 1.0);
        assert_eq!(config.rrf_k, 60);
        assert_eq!(config.max_stream_results, 100);
        assert!(config.enable_vector);
        assert!(config.enable_bm25);
        assert!(config.enable_graph);
    }

    #[test]
    fn test_triple_stream_query_default() {
        let query = TripleStreamQuery::default();
        assert!(query.query_text.is_empty());
        assert!(query.keywords.is_none());
        assert!(query.graph_seed_ids.is_none());
        assert_eq!(query.graph_depth, 2);
        assert_eq!(query.limit, 10);
        assert_eq!(query.offset, 0);
    }

    #[test]
    fn test_ranked_item_creation() {
        let id = Uuid::new_v4();
        let item = RankedItem {
            id,
            score: 0.95,
            rank: 1,
        };
        assert_eq!(item.id, id);
        assert_eq!(item.score, 0.95);
        assert_eq!(item.rank, 1);
    }

    #[test]
    fn test_triple_stream_result_scores() {
        let id = Uuid::new_v4();
        let result = TripleStreamResult {
            id,
            score: 0.5,
            vector_score: Some(0.8),
            bm25_score: Some(0.6),
            graph_score: Some(0.3),
            vector_rank: Some(1),
            bm25_rank: Some(2),
            graph_rank: Some(3),
        };
        assert_eq!(result.id, id);
        assert_eq!(result.vector_score, Some(0.8));
        assert_eq!(result.bm25_score, Some(0.6));
        assert_eq!(result.graph_score, Some(0.3));
    }

    // Integration tests require database setup - these would be run with tokio::test
    // The actual integration tests would verify:
    // 1. Vector search returns relevant results based on embedding similarity
    // 2. BM25 search returns relevant results based on keyword matching
    // 3. Graph traversal returns entities related via semantic relations
    // 4. RRF fusion combines rankings correctly
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::cross_encoder_rerank::CrossEncoderConfig;
    use crate::cross_encoder_rerank::RerankerModelType;
    use crate::recall::RecallService;
    use crate::MemoryBlockType;
    use crate::SemanticEntityType;
    use crate::SemanticRelationType;
    use crate::SqliteMemoryStore;
    use crate::SqliteSemanticStore;

    #[tokio::test]
    async fn test_triple_stream_service_creation() {
        let memory_store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();
        let semantic_store = SqliteSemanticStore::create("sqlite::memory:")
            .await
            .unwrap();

        // Initialize recall schema
        SqliteMemoryStore::init_conversation_logs_schema(memory_store.pool.as_ref())
            .await
            .unwrap();

        let recall_service = RecallService::new(memory_store.clone());

        let service = TripleStreamService::new(memory_store, semantic_store, recall_service);

        assert!(service.config.enable_vector);
        assert!(service.config.enable_bm25);
        assert!(service.config.enable_graph);
    }

    #[tokio::test]
    async fn test_triple_stream_service_with_custom_config() {
        let memory_store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();
        let semantic_store = SqliteSemanticStore::create("sqlite::memory:")
            .await
            .unwrap();

        SqliteMemoryStore::init_conversation_logs_schema(memory_store.pool.as_ref())
            .await
            .unwrap();

        let recall_service = RecallService::new(memory_store.clone());

        let config = TripleStreamConfig {
            vector_weight: 2.0,
            bm25_weight: 1.0,
            graph_weight: 0.5,
            rrf_k: 30,
            max_stream_results: 50,
            enable_vector: true,
            enable_bm25: false,
            enable_graph: true,
            reranker: None,
        };

        let service =
            TripleStreamService::with_config(memory_store, semantic_store, recall_service, config);

        assert_eq!(service.config.vector_weight, 2.0);
        assert!(!service.config.enable_bm25);
    }

    #[tokio::test]
    async fn test_graph_traversal_empty_seeds() {
        let semantic_store = SqliteSemanticStore::create("sqlite::memory:")
            .await
            .unwrap();
        let traversal = GraphTraversal::new(&semantic_store);

        let results = traversal.traverse(&[], 2, None, 10).await.unwrap();

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_graph_traversal_with_entities() {
        let semantic_store = SqliteSemanticStore::create("sqlite::memory:")
            .await
            .unwrap();

        // Create test entities
        let entity1 = crate::SemanticEntity::new(SemanticEntityType::Module, "ModuleA".to_string());
        let entity2 = crate::SemanticEntity::new(SemanticEntityType::Function, "funcA".to_string());
        let entity3 = crate::SemanticEntity::new(SemanticEntityType::Class, "ClassA".to_string());

        semantic_store.store_entity(entity1.clone()).await.unwrap();
        semantic_store.store_entity(entity2.clone()).await.unwrap();
        semantic_store.store_entity(entity3.clone()).await.unwrap();

        // Create relations
        let rel1 =
            crate::SemanticRelation::new(SemanticRelationType::Contains, entity1.id, entity2.id);
        let rel2 =
            crate::SemanticRelation::new(SemanticRelationType::Contains, entity1.id, entity3.id);

        semantic_store.store_relation(rel1).await.unwrap();
        semantic_store.store_relation(rel2).await.unwrap();

        // Traverse from entity1
        let traversal = GraphTraversal::new(&semantic_store);
        let results = traversal
            .traverse(&[entity1.id], 2, None, 10)
            .await
            .unwrap();

        // Should find entity2 and entity3 via Contains relation
        assert!(results.len() >= 2);
        let found_ids: Vec<Uuid> = results.iter().map(|r| r.id).collect();
        assert!(found_ids.contains(&entity2.id));
        assert!(found_ids.contains(&entity3.id));
    }

    #[tokio::test]
    async fn test_find_related_by_type() {
        let semantic_store = SqliteSemanticStore::create("sqlite::memory:")
            .await
            .unwrap();

        // Create entities
        let module = crate::SemanticEntity::new(SemanticEntityType::Module, "ModuleA".to_string());
        let func1 = crate::SemanticEntity::new(SemanticEntityType::Function, "func1".to_string());
        let func2 = crate::SemanticEntity::new(SemanticEntityType::Function, "func2".to_string());

        semantic_store.store_entity(module.clone()).await.unwrap();
        semantic_store.store_entity(func1.clone()).await.unwrap();
        semantic_store.store_entity(func2.clone()).await.unwrap();

        // Create Contains relations
        let rel1 =
            crate::SemanticRelation::new(SemanticRelationType::Contains, module.id, func1.id);
        let rel2 =
            crate::SemanticRelation::new(SemanticRelationType::Contains, module.id, func2.id);

        semantic_store.store_relation(rel1).await.unwrap();
        semantic_store.store_relation(rel2).await.unwrap();

        // Find related entities via Contains relation
        let traversal = GraphTraversal::new(&semantic_store);
        let results = traversal
            .find_related(&[module.id], &[SemanticRelationType::Contains], 10)
            .await
            .unwrap();

        assert_eq!(results.len(), 2);
        let found_ids: Vec<Uuid> = results.iter().map(|r| r.id).collect();
        assert!(found_ids.contains(&func1.id));
        assert!(found_ids.contains(&func2.id));
    }

    #[tokio::test]
    async fn test_vector_search_with_embeddings() {
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        // Create entry with embedding
        let embedding = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let entry = crate::MemoryEntry {
            id: Uuid::new_v4(),
            block_type: crate::MemoryBlockType::Project,
            label: "test-project".to_string(),
            content: "Test content about Rust".to_string(),
            embedding: Some(embedding.clone()),
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
        };

        store.store(entry.clone()).await.unwrap();

        // Search with same embedding (should get high similarity)
        let results = store
            .vector_search(&embedding, "test-repo", 10)
            .await
            .unwrap();

        assert!(!results.is_empty());
        assert_eq!(results[0].id, entry.id);
        assert!(results[0].score > 0.99);
    }

    #[tokio::test]
    async fn test_vector_search_no_embeddings() {
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        // Create entry without embedding
        let entry = crate::MemoryEntry {
            id: Uuid::new_v4(),
            block_type: crate::MemoryBlockType::Project,
            label: "test-project".to_string(),
            content: "Test content".to_string(),
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
        };

        store.store(entry.clone()).await.unwrap();

        // Search with embedding - entry without embedding should not be returned
        let query_embedding = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let results = store
            .vector_search(&query_embedding, "test-repo", 10)
            .await
            .unwrap();

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_rrf_fuse_integration() {
        let rrf = ReciprocalRankFusion::new(60);

        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();
        let id4 = Uuid::new_v4();

        // Three streams with different rankings
        let stream1 = vec![
            RankedItem {
                id: id1,
                score: 1.0,
                rank: 1,
            },
            RankedItem {
                id: id2,
                score: 0.9,
                rank: 2,
            },
            RankedItem {
                id: id3,
                score: 0.8,
                rank: 3,
            },
        ];

        let stream2 = vec![
            RankedItem {
                id: id2,
                score: 1.0,
                rank: 1,
            },
            RankedItem {
                id: id1,
                score: 0.9,
                rank: 2,
            },
            RankedItem {
                id: id4,
                score: 0.7,
                rank: 3,
            },
        ];

        let stream3 = vec![
            RankedItem {
                id: id3,
                score: 1.0,
                rank: 1,
            },
            RankedItem {
                id: id1,
                score: 0.8,
                rank: 2,
            },
            RankedItem {
                id: id4,
                score: 0.6,
                rank: 3,
            },
        ];

        let weights = vec![1.0, 1.0, 1.0];
        let fused = rrf.fuse_ranked(vec![stream1, stream2, stream3], &weights);

        // id1 appears in all streams at ranks 1, 2, 2
        // id2 appears in streams 1, 2 at ranks 2, 1
        // id3 appears in streams 1, 3 at ranks 3, 1
        // id4 appears in streams 2, 3 at ranks 3, 3

        // id1 should be first (appears in all 3 streams)
        assert_eq!(fused[0].0, id1);
        // id2 or id3 should be second
        let second_ids = vec![id2, id3];
        assert!(second_ids.contains(&fused[1].0));
    }

    // =========================================================================
    // Cross-Encoder Reranking Integration Tests
    // =========================================================================

    #[tokio::test]
    async fn test_search_with_reranking_basic() {
        use crate::cross_encoder_rerank::{CrossEncoderConfig, RerankerModelType};

        let memory_store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();
        let semantic_store = SqliteSemanticStore::create("sqlite::memory:")
            .await
            .unwrap();

        SqliteMemoryStore::init_conversation_logs_schema(memory_store.pool.as_ref())
            .await
            .unwrap();

        let recall_service = RecallService::new(memory_store.clone());

        // Store test entries
        let entry1 = crate::MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "Rust error handling".to_string(),
            content: "This module handles errors in Rust using the Result type.".to_string(),
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
        };

        let entry2 = crate::MemoryEntry {
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
        };

        memory_store.store(entry1.clone()).await.unwrap();
        memory_store.store(entry2.clone()).await.unwrap();

        let service =
            TripleStreamService::new(memory_store.clone(), semantic_store, recall_service);

        // Configure reranking
        let mut reranker_config = CrossEncoderConfig::default();
        reranker_config.model_type = RerankerModelType::Simple;
        reranker_config.max_candidates = 10;
        reranker_config.max_results = 10;

        let query = TripleStreamQuery {
            query_text: "Rust error handling".to_string(),
            repository: "test-repo".to_string(),
            limit: 10,
            ..Default::default()
        };

        // Search with reranking
        let results = service
            .search_with_reranking(query, Some(reranker_config))
            .await
            .unwrap();

        // Should return results (at least the Rust entry should be ranked higher)
        assert!(!results.is_empty());
    }

    #[tokio::test]
    async fn test_search_with_reranking_disabled() {
        let memory_store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();
        let semantic_store = SqliteSemanticStore::create("sqlite::memory:")
            .await
            .unwrap();

        SqliteMemoryStore::init_conversation_logs_schema(memory_store.pool.as_ref())
            .await
            .unwrap();

        let recall_service = RecallService::new(memory_store.clone());

        // Store a test entry
        let entry = crate::MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "Test project".to_string(),
            content: "Test content".to_string(),
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
        };

        memory_store.store(entry.clone()).await.unwrap();

        let service =
            TripleStreamService::new(memory_store.clone(), semantic_store, recall_service);

        // Disable reranking by passing None
        let query = TripleStreamQuery {
            query_text: "test".to_string(),
            repository: "test-repo".to_string(),
            limit: 10,
            ..Default::default()
        };

        let results = service.search_with_reranking(query, None).await.unwrap();

        // Should still return results (reranking disabled)
        assert!(!results.is_empty());
    }

    #[tokio::test]
    async fn test_search_with_reranking_no_results() {
        let memory_store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();
        let semantic_store = SqliteSemanticStore::create("sqlite::memory:")
            .await
            .unwrap();

        SqliteMemoryStore::init_conversation_logs_schema(memory_store.pool.as_ref())
            .await
            .unwrap();

        let recall_service = RecallService::new(memory_store.clone());

        let service =
            TripleStreamService::new(memory_store.clone(), semantic_store, recall_service);

        let reranker_config = CrossEncoderConfig::default();

        // Query for non-existent content
        let query = TripleStreamQuery {
            query_text: "nonexistent content xyz123".to_string(),
            repository: "test-repo".to_string(),
            limit: 10,
            ..Default::default()
        };

        let results = service
            .search_with_reranking(query, Some(reranker_config))
            .await
            .unwrap();

        // Should return empty when no matching content
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_triple_stream_config_with_reranker() {
        let mut config = TripleStreamConfig::default();
        assert!(config.reranker.is_some());

        // Disable reranker
        config.reranker = None;
        assert!(config.reranker.is_none());

        // Re-enable with custom config
        let reranker_config = CrossEncoderConfig {
            enabled: true,
            max_candidates: 25,
            max_results: 5,
            model_type: RerankerModelType::Mock,
            ..Default::default()
        };
        config.reranker = Some(reranker_config);
        assert!(config.reranker.is_some());
        let reranker = config.reranker.unwrap();
        assert_eq!(reranker.max_candidates, 25);
        assert_eq!(reranker.max_results, 5);
        assert_eq!(reranker.model_type, RerankerModelType::Mock);
    }
}
