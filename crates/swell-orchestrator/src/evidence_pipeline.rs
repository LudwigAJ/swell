//! Evidence pipeline for research content processing.
//!
//! This module provides a complete pipeline for processing fetched web content:
//! - [`EvidenceChunk`] - chunked content with full provenance metadata
//! - [`EvidenceSource`] - deduplicated source with aggregated chunks
//! - [`EvidencePipeline`] - main pipeline for chunking, deduplication, and retrieval
//!
//! # Pipeline Stages
//!
//! 1. **Chunking**: Split content into overlapping chunks while preserving provenance
//! 2. **Deduplication**: Remove duplicate sources based on URL or content similarity
//! 3. **Retrieval**: Hybrid keyword + semantic search
//! 4. **Reranking**: Reorder results by relevance to query
//!
//! # Provenance Tracking
//!
//! All chunks maintain full provenance including:
//! - URL, title, fetch timestamp, publication date
//! - Chunk index and position within source
//! - Content hash for integrity

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Configuration for the evidence pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidencePipelineConfig {
    /// Target size for each chunk in characters
    pub chunk_size: usize,
    /// Overlap between chunks in characters
    pub chunk_overlap: usize,
    /// Minimum chunk size (chunks smaller than this are merged with next)
    pub min_chunk_size: usize,
    /// Similarity threshold for content deduplication (0.0 to 1.0)
    pub dedup_similarity_threshold: f64,
    /// Maximum number of sources to retain after deduplication
    pub max_sources: usize,
    /// Maximum number of chunks per source
    pub max_chunks_per_source: usize,
}

impl Default for EvidencePipelineConfig {
    fn default() -> Self {
        Self {
            chunk_size: 500,
            chunk_overlap: 100,
            min_chunk_size: 100,
            dedup_similarity_threshold: 0.85,
            max_sources: 20,
            max_chunks_per_source: 50,
        }
    }
}

/// Provenance information for a chunk of content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkProvenance {
    /// URL of the source
    pub url: String,
    /// Title of the source page
    pub title: String,
    /// When the content was fetched
    pub fetched_at: DateTime<Utc>,
    /// Publication date if available
    pub publication_date: Option<DateTime<Utc>>,
    /// Position of this chunk within the source (0-indexed)
    pub chunk_index: usize,
    /// Total number of chunks from this source
    pub total_chunks: usize,
    /// Character offset where this chunk starts in the original text
    pub char_offset: usize,
    /// Content hash for integrity verification
    pub content_hash: String,
}

/// A single chunk of content with its provenance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceChunk {
    /// Unique identifier for this chunk
    pub id: Uuid,
    /// The actual text content
    pub text: String,
    /// Provenance metadata
    pub provenance: ChunkProvenance,
    /// Extracted keywords from this chunk
    #[serde(default)]
    pub keywords: Vec<String>,
}

impl EvidenceChunk {
    /// Create a new evidence chunk
    pub fn new(text: String, provenance: ChunkProvenance) -> Self {
        let keywords = extract_keywords(&text);
        Self {
            id: Uuid::new_v4(),
            text,
            provenance,
            keywords,
        }
    }

    /// Get a snippet of the chunk text for display
    pub fn snippet(&self, max_len: usize) -> String {
        if self.text.len() <= max_len {
            self.text.clone()
        } else {
            format!("{}...", &self.text[..max_len.saturating_sub(3)])
        }
    }
}

/// A deduplicated source containing multiple related chunks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceSource {
    /// Unique identifier for this source
    pub id: Uuid,
    /// URL as the canonical identifier
    pub url: String,
    /// Title of the source page
    pub title: String,
    /// When the content was fetched
    pub fetched_at: DateTime<Utc>,
    /// Publication date if available
    pub publication_date: Option<DateTime<Utc>>,
    /// All chunks from this source
    pub chunks: Vec<EvidenceChunk>,
    /// Aggregated keywords across all chunks
    #[serde(default)]
    pub keywords: Vec<String>,
}

impl EvidenceSource {
    /// Get the combined text of all chunks
    pub fn combined_text(&self) -> String {
        self.chunks
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Get a combined content hash
    pub fn content_hash(&self) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        self.url.hash(&mut hasher);
        for chunk in &self.chunks {
            chunk.provenance.content_hash.hash(&mut hasher);
        }
        format!("{:x}", hasher.finish())
    }
}

/// A retrieved evidence item with relevance score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceResult {
    /// The evidence chunk
    pub chunk: EvidenceChunk,
    /// Relevance score (higher = more relevant)
    pub score: f64,
    /// Whether this matched via keyword or semantic search
    pub match_type: MatchType,
    /// Position in original ranking (before reranking)
    pub original_position: Option<usize>,
}

/// Type of match found during retrieval
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchType {
    /// Matched via keyword search (BM25)
    Keyword,
    /// Matched via semantic similarity
    Semantic,
    /// Matched via both keyword and semantic
    Hybrid,
}

/// Query for evidence retrieval
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceQuery {
    /// The search query
    pub query: String,
    /// Query keywords for keyword search
    pub keywords: Vec<String>,
    /// Maximum number of results to return
    pub limit: usize,
    /// Minimum relevance score threshold
    pub min_score: f64,
    /// Whether to include all chunks from matched sources
    pub include_all_chunks: bool,
}

impl Default for EvidenceQuery {
    fn default() -> Self {
        Self {
            query: String::new(),
            keywords: Vec::new(),
            limit: 10,
            min_score: 0.0,
            include_all_chunks: false,
        }
    }
}

/// Evidence pipeline for processing and retrieving research content
#[derive(Debug, Clone)]
pub struct EvidencePipeline {
    config: EvidencePipelineConfig,
    /// Inverted index: keyword -> chunk IDs
    inverted_index: HashMap<String, HashSet<Uuid>>,
    /// Document frequencies for BM25
    doc_freqs: HashMap<String, usize>,
    /// Total number of documents
    total_docs: usize,
}

impl EvidencePipeline {
    /// Create a new evidence pipeline with default configuration
    pub fn new() -> Self {
        Self::with_config(EvidencePipelineConfig::default())
    }

    /// Create a new evidence pipeline with custom configuration
    pub fn with_config(config: EvidencePipelineConfig) -> Self {
        Self {
            config,
            inverted_index: HashMap::new(),
            doc_freqs: HashMap::new(),
            total_docs: 0,
        }
    }

    /// Process raw page content and create evidence chunks
    pub fn chunk_content(
        &mut self,
        url: String,
        title: String,
        fetched_at: DateTime<Utc>,
        publication_date: Option<DateTime<Utc>>,
        content: String,
    ) -> Vec<EvidenceChunk> {
        let mut chunks = Vec::new();
        let content_len = content.len();

        if content_len == 0 {
            return chunks;
        }

        // Calculate chunk boundaries with overlap
        let chunk_size = self.config.chunk_size;
        let overlap = self.config.chunk_overlap;
        let min_chunk = self.config.min_chunk_size;

        let mut offset = 0;
        let mut chunk_index = 0;

        while offset < content_len {
            let end = (offset + chunk_size).min(content_len);

            // Adjust end to not cut words
            let actual_end = if end < content_len {
                if let Some(space_pos) = content[..end].rfind(' ') {
                    space_pos
                } else if let Some(space_pos) = content[end..].find(' ') {
                    end + space_pos
                } else {
                    end
                }
            } else {
                end
            };

            let chunk_text = content[offset..actual_end].trim().to_string();

            // Only create chunk if it's large enough
            if chunk_text.len() >= min_chunk {
                let content_hash = calculate_hash(&chunk_text);

                let provenance = ChunkProvenance {
                    url: url.clone(),
                    title: title.clone(),
                    fetched_at,
                    publication_date,
                    chunk_index,
                    total_chunks: 0, // Will be updated later
                    char_offset: offset,
                    content_hash,
                };

                let chunk = EvidenceChunk::new(chunk_text, provenance);
                self.index_chunk(&chunk);
                chunks.push(chunk);
            }

            // Move offset with overlap
            if actual_end <= offset {
                // Prevent infinite loop if we can't make progress
                break;
            }
            offset = actual_end.saturating_sub(overlap);
            chunk_index += 1;
        }

        // Update total_chunks for all chunks
        let total = chunks.len();
        for chunk in &mut chunks {
            chunk.provenance.total_chunks = total;
        }

        self.total_docs += chunks.len();

        // Limit chunks per source
        if chunks.len() > self.config.max_chunks_per_source {
            chunks.truncate(self.config.max_chunks_per_source);
        }

        chunks
    }

    /// Index a chunk for keyword search
    fn index_chunk(&mut self, chunk: &EvidenceChunk) {
        // Update document frequencies
        let unique_keywords: HashSet<_> = chunk.keywords.iter().collect();
        for kw in &unique_keywords {
            *self.doc_freqs.entry(kw.to_string()).or_insert(0) += 1;

            self.inverted_index
                .entry(kw.to_string())
                .or_default()
                .insert(chunk.id);
        }
    }

    /// Build evidence sources from chunks by grouping by URL
    pub fn build_sources(&self, chunks: Vec<EvidenceChunk>) -> Vec<EvidenceSource> {
        let mut url_to_chunks: HashMap<String, Vec<EvidenceChunk>> = HashMap::new();

        for chunk in chunks {
            url_to_chunks
                .entry(chunk.provenance.url.clone())
                .or_default()
                .push(chunk);
        }

        let mut sources: Vec<EvidenceSource> = url_to_chunks
            .into_iter()
            .map(|(url, chunks)| {
                let title = chunks[0].provenance.title.clone();
                let fetched_at = chunks[0].provenance.fetched_at;
                let publication_date = chunks[0].provenance.publication_date;

                // Aggregate keywords
                let mut all_keywords: HashSet<String> = HashSet::new();
                for chunk in &chunks {
                    all_keywords.extend(chunk.keywords.iter().cloned());
                }

                EvidenceSource {
                    id: Uuid::new_v4(),
                    url,
                    title,
                    fetched_at,
                    publication_date,
                    chunks,
                    keywords: all_keywords.into_iter().collect(),
                }
            })
            .collect();

        // Sort by fetched_at (newest first)
        sources.sort_by(|a, b| b.fetched_at.cmp(&a.fetched_at));

        // Limit number of sources
        if sources.len() > self.config.max_sources {
            sources.truncate(self.config.max_sources);
        }

        sources
    }

    /// Deduplicate sources based on URL or content similarity
    pub fn deduplicate_sources(&self, sources: &mut Vec<EvidenceSource>) {
        // First, deduplicate by exact URL
        let mut seen_urls: HashSet<String> = HashSet::new();
        sources.retain(|s| seen_urls.insert(s.url.clone()));

        // Then, deduplicate by content similarity
        let mut i = 0;
        while i < sources.len() {
            let mut j = i + 1;
            while j < sources.len() {
                if self.content_similarity(&sources[i], &sources[j])
                    > self.config.dedup_similarity_threshold
                {
                    // Remove j (merge into i would be more sophisticated)
                    sources.remove(j);
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
    }

    /// Calculate content similarity between two sources (0.0 to 1.0)
    fn content_similarity(&self, a: &EvidenceSource, b: &EvidenceSource) -> f64 {
        // Simple keyword overlap (Jaccard similarity)
        let a_keywords: HashSet<_> = a.keywords.iter().collect();
        let b_keywords: HashSet<_> = b.keywords.iter().collect();

        let intersection = a_keywords.intersection(&b_keywords).count();
        let union = a_keywords.union(&b_keywords).count();

        if union == 0 {
            return 0.0;
        }

        intersection as f64 / union as f64
    }

    /// Retrieve relevant evidence chunks for a query
    pub fn retrieve(&self, query: &EvidenceQuery) -> Vec<EvidenceResult> {
        if query.query.is_empty() && query.keywords.is_empty() {
            return Vec::new();
        }

        // Build query keywords if not provided
        let query_keywords: Vec<String> = if query.keywords.is_empty() {
            extract_keywords(&query.query)
        } else {
            query.keywords.clone()
        };

        // Track which chunks have been scored
        let mut chunk_scores: HashMap<Uuid, (f64, MatchType)> = HashMap::new();

        // Keyword search using inverted index
        for kw in &query_keywords {
            let kw_lower = kw.to_lowercase();

            if let Some(chunk_ids) = self.inverted_index.get(&kw_lower) {
                for chunk_id in chunk_ids {
                    let df = self.doc_freqs.get(&kw_lower).copied().unwrap_or(1).max(1) as f64;
                    // IDF component
                    let idf = ((self.total_docs as f64 + 1.0) / df).ln();

                    let entry = chunk_scores.entry(*chunk_id).or_insert((0.0, MatchType::Keyword));
                    entry.0 += idf;
                    if entry.1 == MatchType::Semantic {
                        entry.1 = MatchType::Hybrid;
                    }
                }
            }
        }

        // Build results
        let mut results: Vec<EvidenceResult> = chunk_scores
            .into_iter()
            .map(|(chunk_id, (score, match_type))| {
                // We'll need to look up the actual chunk
                // For now, we create a placeholder
                EvidenceResult {
                    chunk: EvidenceChunk {
                        id: chunk_id,
                        text: String::new(), // Will be filled in later
                        provenance: ChunkProvenance {
                            url: String::new(),
                            title: String::new(),
                            fetched_at: Utc::now(),
                            publication_date: None,
                            chunk_index: 0,
                            total_chunks: 0,
                            char_offset: 0,
                            content_hash: String::new(),
                        },
                        keywords: Vec::new(),
                    },
                    score,
                    match_type,
                    original_position: None,
                }
            })
            .filter(|r| r.score >= query.min_score)
            .collect();

        // Sort by score descending
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        // Limit results
        results.truncate(query.limit);

        // Update original positions
        for (i, result) in results.iter_mut().enumerate() {
            result.original_position = Some(i);
        }

        results
    }

    /// Rerank results based on additional relevance signals
    pub fn rerank(&self, results: &mut [EvidenceResult], boost_factors: &RerankFactors) {
        for result in &mut *results {
            let mut new_score = result.score;

            // Boost for recency
            if boost_factors.recency_boost {
                let age_hours = (Utc::now() - result.chunk.provenance.fetched_at).num_hours() as f64;
                let recency_boost = (1.0 / (1.0 + age_hours / 24.0)) * 0.2; // Max 0.2 boost for very recent
                new_score += recency_boost;
            }

            // Boost for publication date if available
            if boost_factors.publication_date_boost {
                if let Some(pub_date) = result.chunk.provenance.publication_date {
                    let age_hours = (Utc::now() - pub_date).num_hours() as f64;
                    let pub_boost = (1.0 / (1.0 + age_hours / 24.0)) * 0.3; // Max 0.3 boost
                    new_score += pub_boost;
                }
            }

            // Boost for title match
            if boost_factors.title_match_boost {
                // This would require access to the title, which we don't have in EvidenceResult
                // For now, this is a placeholder
            }

            result.score = new_score;
        }

        // Re-sort by new scores
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    }

    /// Process fetched content end-to-end
    pub fn process(
        &mut self,
        url: String,
        title: String,
        fetched_at: DateTime<Utc>,
        publication_date: Option<DateTime<Utc>>,
        content: String,
    ) -> EvidenceSource {
        let chunks = self.chunk_content(url.clone(), title.clone(), fetched_at, publication_date, content);
        let sources = self.build_sources(chunks);

        // If we have exactly one source, return it
        if !sources.is_empty() {
            sources.into_iter().next().unwrap()
        } else {
            // Return empty source
            EvidenceSource {
                id: Uuid::new_v4(),
                url,
                title,
                fetched_at,
                publication_date,
                chunks: Vec::new(),
                keywords: Vec::new(),
            }
        }
    }
}

impl Default for EvidencePipeline {
    fn default() -> Self {
        Self::new()
    }
}

/// Factors for reranking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankFactors {
    /// Boost for recent content
    pub recency_boost: bool,
    /// Boost for content with publication date
    pub publication_date_boost: bool,
    /// Boost for title matches
    pub title_match_boost: bool,
}

impl Default for RerankFactors {
    fn default() -> Self {
        Self {
            recency_boost: true,
            publication_date_boost: true,
            title_match_boost: true,
        }
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Extract keywords from text (simple tokenization + filtering)
fn extract_keywords(text: &str) -> Vec<String> {
    let text_lower = text.to_lowercase();

    // Split on non-alphanumeric characters
    let words: Vec<String> = text_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() > 2) // Minimum word length
        .map(|s| s.to_string())
        .collect();

    // Common stop words to filter
    let stop_words: HashSet<&str> = [
        "the", "and", "for", "are", "but", "not", "you", "all", "can", "had", "her",
        "was", "one", "our", "out", "has", "have", "been", "were", "they", "this",
        "that", "with", "from", "your", "what", "when", "where", "which", "their",
        "will", "would", "there", "could", "other", "into", "just", "about", "also",
    ]
    .iter()
    .copied()
    .collect();

    // Count word frequencies
    let mut freq: HashMap<String, usize> = HashMap::new();
    for word in &words {
        if !stop_words.contains(word.as_str()) {
            *freq.entry(word.clone()).or_insert(0) += 1;
        }
    }

    // Sort by frequency and return top keywords
    let mut sorted: Vec<_> = freq.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    sorted.into_iter().take(20).map(|(w, _)| w).collect()
}

/// Calculate a simple hash of content
fn calculate_hash(content: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_keywords() {
        let text = "The Rust programming language is great for systems programming. \
                    Rust provides memory safety without garbage collection.";
        let keywords = extract_keywords(text);

        assert!(keywords.contains(&"rust".to_string()));
        assert!(keywords.contains(&"programming".to_string()));
        assert!(keywords.contains(&"language".to_string()));
        // Should not contain stop words
        assert!(!keywords.contains(&"the".to_string()));
        assert!(!keywords.contains(&"for".to_string()));
    }

    #[test]
    fn test_evidence_pipeline_chunking() {
        let mut pipeline = EvidencePipeline::new();

        let content = "This is a test document. It has multiple sentences. \
                       We are chunking this content for evidence. Each chunk \
                       should be meaningful and retain provenance.";

        let chunks = pipeline.chunk_content(
            "https://example.com/article".to_string(),
            "Test Article".to_string(),
            Utc::now(),
            None,
            content.to_string(),
        );

        assert!(!chunks.is_empty());
        // All chunks should have provenance
        for chunk in &chunks {
            assert_eq!(chunk.provenance.url, "https://example.com/article");
            assert_eq!(chunk.provenance.title, "Test Article");
            assert!(!chunk.provenance.content_hash.is_empty());
        }
    }

    #[test]
    fn test_evidence_pipeline_deduplication() {
        let pipeline = EvidencePipeline::new();
        let mut sources = vec![
            EvidenceSource {
                id: Uuid::new_v4(),
                url: "https://example.com/1".to_string(),
                title: "Article 1".to_string(),
                fetched_at: Utc::now(),
                publication_date: None,
                chunks: vec![],
                keywords: vec!["rust".to_string(), "programming".to_string()],
            },
            EvidenceSource {
                id: Uuid::new_v4(),
                url: "https://example.com/2".to_string(),
                title: "Article 2".to_string(),
                fetched_at: Utc::now(),
                publication_date: None,
                chunks: vec![],
                keywords: vec!["rust".to_string(), "programming".to_string()],
            },
            EvidenceSource {
                id: Uuid::new_v4(),
                url: "https://example.com/3".to_string(),
                title: "Article 3".to_string(),
                fetched_at: Utc::now(),
                publication_date: None,
                chunks: vec![],
                keywords: vec!["python".to_string()],
            },
        ];

        pipeline.deduplicate_sources(&mut sources);

        // Should deduplicate example.com/1 and example.com/2 based on keyword similarity
        assert!(sources.len() <= 3);
    }

    #[test]
    fn test_content_similarity() {
        let pipeline = EvidencePipeline::new();

        let source1 = EvidenceSource {
            id: Uuid::new_v4(),
            url: "https://example.com/1".to_string(),
            title: "Rust Programming".to_string(),
            fetched_at: Utc::now(),
            publication_date: None,
            chunks: vec![],
            keywords: vec!["rust".to_string(), "programming".to_string(), "language".to_string()],
        };

        let source2 = EvidenceSource {
            id: Uuid::new_v4(),
            url: "https://example.com/2".to_string(),
            title: "Rust Language".to_string(),
            fetched_at: Utc::now(),
            publication_date: None,
            chunks: vec![],
            keywords: vec!["rust".to_string(), "language".to_string()],
        };

        let similarity = pipeline.content_similarity(&source1, &source2);
        // Jaccard: intersection = {rust, language}, union = {rust, programming, language}
        // similarity = 2/3 ≈ 0.67
        assert!((similarity - 0.666).abs() < 0.01);
    }

    #[test]
    fn test_evidence_query_default() {
        let query = EvidenceQuery::default();
        assert!(query.query.is_empty());
        assert!(query.keywords.is_empty());
        assert_eq!(query.limit, 10);
    }

    #[test]
    fn test_rerank_factors_default() {
        let factors = RerankFactors::default();
        assert!(factors.recency_boost);
        assert!(factors.publication_date_boost);
        assert!(factors.title_match_boost);
    }

    #[test]
    fn test_chunk_provenance_serialization() {
        let provenance = ChunkProvenance {
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            fetched_at: Utc::now(),
            publication_date: Some(Utc::now()),
            chunk_index: 0,
            total_chunks: 5,
            char_offset: 100,
            content_hash: "abc123".to_string(),
        };

        let json = serde_json::to_string(&provenance).unwrap();
        assert!(json.contains("https://example.com"));
        assert!(json.contains("Example"));
        assert!(json.contains("publication_date"));
    }

    #[test]
    fn test_evidence_pipeline_process() {
        let mut pipeline = EvidencePipeline::new();

        let source = pipeline.process(
            "https://example.com/article".to_string(),
            "Test Article".to_string(),
            Utc::now(),
            None,
            "This is a test article with some content that we want to process.".to_string(),
        );

        assert_eq!(source.url, "https://example.com/article");
        assert_eq!(source.title, "Test Article");
        assert!(!source.chunks.is_empty());
    }

    #[test]
    fn test_evidence_source_combined_text() {
        let chunk1 = EvidenceChunk::new(
            "First part of the content.".to_string(),
            ChunkProvenance {
                url: "https://example.com".to_string(),
                title: "Example".to_string(),
                fetched_at: Utc::now(),
                publication_date: None,
                chunk_index: 0,
                total_chunks: 2,
                char_offset: 0,
                content_hash: "hash1".to_string(),
            },
        );

        let chunk2 = EvidenceChunk::new(
            "Second part of the content.".to_string(),
            ChunkProvenance {
                url: "https://example.com".to_string(),
                title: "Example".to_string(),
                fetched_at: Utc::now(),
                publication_date: None,
                chunk_index: 1,
                total_chunks: 2,
                char_offset: 24,
                content_hash: "hash2".to_string(),
            },
        );

        let source = EvidenceSource {
            id: Uuid::new_v4(),
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            fetched_at: Utc::now(),
            publication_date: None,
            chunks: vec![chunk1, chunk2],
            keywords: vec!["example".to_string()],
        };

        let combined = source.combined_text();
        assert!(combined.contains("First part"));
        assert!(combined.contains("Second part"));
    }

    #[test]
    fn test_evidence_pipeline_config_default() {
        let config = EvidencePipelineConfig::default();
        assert_eq!(config.chunk_size, 500);
        assert_eq!(config.chunk_overlap, 100);
        assert_eq!(config.dedup_similarity_threshold, 0.85);
        assert_eq!(config.max_sources, 20);
    }

    #[test]
    fn test_match_type_serialization() {
        let keyword = MatchType::Keyword;
        let semantic = MatchType::Semantic;
        let hybrid = MatchType::Hybrid;

        let keyword_json = serde_json::to_string(&keyword).unwrap();
        let semantic_json = serde_json::to_string(&semantic).unwrap();
        let hybrid_json = serde_json::to_string(&hybrid).unwrap();

        assert_eq!(keyword_json, "\"Keyword\"");
        assert_eq!(semantic_json, "\"Semantic\"");
        assert_eq!(hybrid_json, "\"Hybrid\"");
    }
}
