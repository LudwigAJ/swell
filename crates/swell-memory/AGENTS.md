# swell-memory AGENTS.md

## Purpose

`swell-memory` provides a SQLite-based memory system for the SWELL autonomous coding engine. It offers persistent memory storage with support for semantic search, knowledge graphs, pattern learning, and recall capabilities.

This crate handles:
- **SqliteMemoryStore** — SQLite-backed memory storage with repository-scoped isolation
- **Knowledge Graph** — Property graph for code structure with typed nodes and edges
- **Recall** — BM25 keyword search and temporal queries for conversation logs
- **TripleStream** — Vector + BM25 + Graph traversal with Reciprocal Rank Fusion
- **CrossEncoderReranker** — BGE-reranker for cross-encoder reranking
- **SkillExtraction** — Extracts reusable procedures from successful task trajectories
- **PatternLearning** — Learns anti-patterns from rejection feedback
- **ContrastiveLearning** — Analyzes success/failure trajectories
- **OperatorFeedback** — Parses CLAUDE.md/AGENTS.md with higher trust weight
- **Semantic Memory** — Facts, entities, and relationships as graph nodes
- **Procedural Memory** — Strategies and action patterns with confidence scoring
- **MetaCognitive** — Self-knowledge for model performance tracking
- **Decay** — Time-based decay with different rates per memory type
- **ConflictResolution** — Detects and resolves contradictory memories
- **GoldenSampleTesting** — Validates learned procedures against test cases

**Depends on:** `swell-core` (for `MemoryStore` trait, `MemoryEntry`, `SwellError`)

## Public API

### Memory Store

```rust
pub struct SqliteMemoryStore {
    pool: Arc<SqlitePool>,
}

impl SqliteMemoryStore {
    pub async fn new(database_url: &str) -> Result<Self, SwellError>;
    pub async fn create(database_url: &str) -> Result<Self, SwellError>;
    pub async fn reinforce(&self, id: Uuid) -> Result<(), SwellError>;
    pub async fn mark_stale(&self, id: Uuid) -> Result<(), SwellError>;
    pub async fn update_staleness_status(&self, staleness_window_days: i64) -> Result<Vec<Uuid>, SwellError>;
    pub async fn get_stale_memories(&self) -> Result<Vec<MemoryEntry>, SwellError>;
    pub async fn is_memory_stale(&self, id: Uuid) -> Result<bool, SwellError>;
}

#[async_trait]
impl MemoryStore for SqliteMemoryStore {
    async fn store(&self, entry: MemoryEntry) -> Result<Uuid, SwellError>;
    async fn get(&self, id: Uuid) -> Result<Option<MemoryEntry>, SwellError>;
    async fn update(&self, entry: MemoryEntry) -> Result<(), SwellError>;
    async fn delete(&self, id: Uuid) -> Result<(), SwellError>;
    async fn search(&self, query: MemoryQuery) -> Result<Vec<MemorySearchResult>, SwellError>;
    async fn get_by_type(&self, block_type: MemoryBlockType, repository: String) -> Result<Vec<MemoryEntry>, SwellError>;
    async fn get_by_label(&self, label: String, repository: String) -> Result<Vec<MemoryEntry>, SwellError>;
    async fn get_by_provenance(&self, source_episode_id: Uuid, repository: String) -> Result<Vec<MemoryEntry>, SwellError>;
}
```

### Knowledge Graph

```rust
pub struct KnowledgeGraphNode {
    pub id: String,
    pub node_type: String,
    pub name: String,
    pub properties: serde_json::Value,
}

pub struct KnowledgeGraphEdge {
    pub source: String,
    pub target: String,
    pub edge_type: String,
    pub properties: serde_json::Value,
}

pub enum KnowledgeGraphQuery { /* ... */ }

pub struct SqliteKnowledgeGraph { /* ... */ }

impl SqliteKnowledgeGraph {
    pub async fn new(database_url: &str) -> Result<Self, SwellError>;
    pub async fn insert_node(&self, node: KnowledgeGraphNode) -> Result<(), SwellError>;
    pub async fn insert_edge(&self, edge: KnowledgeGraphEdge) -> Result<(), SwellError>;
    pub async fn query(&self, query: KnowledgeGraphQuery) -> Result<Vec<CrossReferenceResult>, SwellError>;
}
```

### Recall

```rust
pub mod recall { /* BM25 keyword search and temporal queries */ }
```

### Triple Stream Retrieval

```rust
pub struct TripleStreamConfig { /* ... */ }

pub struct TripleStreamQuery { /* ... */ }

pub struct TripleStreamResult {
    pub entry: MemoryEntry,
    pub bm25_score: f32,
    pub vector_score: f32,
    pub graph_score: f32,
    pub fused_score: f32,
}

pub struct TripleStreamService { /* ... */ }

pub struct ReciprocalRankFusion { /* ... */ }

pub struct GraphTraversal { /* ... */ }
```

### Cross-Encoder Reranking

```rust
pub enum RerankerModelType { BGE, Mock }

pub struct CrossEncoderConfig { /* ... */ }

pub struct RerankCandidate {
    pub id: Uuid,
    pub score: f32,
}

pub struct RerankResult {
    pub reranked: Vec<RerankCandidate>,
    pub model_type: RerankerModelType,
}

pub trait CrossEncoderReranker: Send + Sync {
    async fn rerank(&self, candidates: Vec<RerankCandidate>, query: &str) -> Result<RerankResult, SwellError>;
}

pub struct CrossEncoderService { /* ... */ }
pub struct MockReranker { /* ... */ }
pub struct SimpleReranker { /* ... */ }
```

### Contrastive Learning

```rust
pub enum PairType { Success, Failure }

pub enum StepStatus { Succeeded, Failed, Partial }

pub struct ContrastivePair {
    pub pair_type: PairType,
    pub trajectory_id: String,
    pub steps: Vec<TrajectoryStep>,
}

pub struct ContrastiveLearningConfig { /* ... */ }

pub struct ContrastiveLearningResult {
    pub loss: f32,
    pub components: LossComponents,
    pub pairs_analyzed: usize,
}

pub struct ContrastiveLearningService { /* ... */ }
pub struct ContrastiveAnalyzer { /* ... */ }
pub struct ContrastiveTrainer { /* ... */ }
```

### Skill Extraction

```rust
pub mod skill_extraction { /* ... */ }
```

### Pattern Learning

```rust
pub mod pattern_learning { /* ... */ }
```

### Golden Sample Testing

```rust
pub enum GoldenSampleSource { Extracted, UserProvided }

pub struct GoldenSample {
    pub id: Uuid,
    pub name: String,
    pub source: GoldenSampleSource,
    pub procedure: String,
    pub test_cases: Vec<String>,
    pub validation_result: Option<GoldenSampleValidationResult>,
}

pub struct GoldenSampleTester { /* ... */ }
pub struct GoldenSampleService { /* ... */ }
```

### Decay and Staleness

```rust
pub enum DecayRate { Procedural, Environmental, Buffer }

pub struct DecayedScore {
    pub original_score: f32,
    pub decayed_score: f32,
    pub days_elapsed: i64,
    pub decay_rate: DecayRate,
}

pub const procedural_decay_rate: f32 = 0.99;
pub const environmental_decay_rate: f32 = 0.95;
pub const buffer_decay_rate: f32 = 0.90;

pub fn calculate_decay(original_score: f32, days_elapsed: i64, block_type: MemoryBlockType) -> f32;
pub fn decay_rate_for_block_type(block_type: MemoryBlockType) -> f32;
```

### Conflict Resolution

```rust
pub enum ConflictType { Semantic, Temporal, Structural }

pub enum ResolutionStrategy { Newest, HighestConfidence, Merge, Manual }

pub struct MemoryConflict {
    pub id: Uuid,
    pub block_type: MemoryBlockType,
    pub conflicting_entries: Vec<ConflictMemoryInfo>,
    pub conflict_type: ConflictType,
}

pub struct ConflictResolutionResult {
    pub resolved: bool,
    pub resolution: Option<MemoryEntry>,
    pub strategy: ResolutionStrategy,
    pub confidence: f32,
}

pub trait MemoryConflictResolver: Send + Sync {
    fn resolve(&self, conflict: &MemoryConflict) -> ConflictResolutionResult;
}

pub struct ConflictResolutionService { /* ... */ }
pub struct MemoryConflictDetector { /* ... */ }
```

### Key Re-exports

```rust
pub use blocks::{MemoryBlock, MemoryBlockManager};
pub use event_log::{AppendOnlyLog, EventLogEntry};
pub use knowledge_graph::{KnowledgeGraphEdge, KnowledgeGraphNode, SqliteKnowledgeGraph};
pub use recall::RecallService;
pub use triple_stream::{ReciprocalRankFusion, TripleStreamConfig, TripleStreamResult, TripleStreamService};
pub use cross_encoder_rerank::{CrossEncoderConfig, CrossEncoderReranker, CrossEncoderService, MockReranker, RerankResult};
pub use semantic::{SemanticEntity, SemanticRelation, SqliteSemanticStore};
pub use procedural::{BetaPosterior, ConfidenceLevel, Procedure, SqliteProceduralStore};
pub use meta_cognitive::{MetaCognitiveStore, ModelPerformance, PromptingStrategy, SqliteMetaCognitiveStore};
pub use contrastive_learning::{ContrastiveAnalyzer, ContrastiveLearningService, ContrastivePair, ContrastiveTrainer};
pub use operator_feedback::{OperatorFeedbackParser, OperatorFeedbackService, OperatorGuidancePattern};
pub use golden_sample_testing::{GoldenSample, GoldenSampleService, GoldenSampleSource, GoldenSampleTester};
pub use pattern_learning::PatternLearner;
pub use skill_extraction::SkillExtractor;
pub use decay::{calculate_decay, DecayRate, ProceduralRate};
pub use conflict_resolution::{ConflictResolutionService, MemoryConflictDetector, MemoryConflictResolver};
pub use staleness::{is_stale_memory, StalenessConfig};
pub use version_rollback::{MemoryVersion, RollbackResult};
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                       swell-memory                                  │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │                    SqliteMemoryStore                         │   │
│  │  (Core storage with repository-scoped isolation)              │   │
│  └──────────────────────────────────────────────────────────────┘   │
│                              │                                      │
│  ┌───────────────────────────┼───────────────────────────┐          │
│  │                           ▼                           │          │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐    │          │
│  │  │   Recall    │  │   Triple    │  │ Knowledge   │    │          │
│  │  │  (BM25 +    │  │   Stream    │  │   Graph     │    │          │
│  │  │  temporal) │  │  (RRF +     │  │ (nodes +    │    │          │
│  │  │             │  │  graph)     │  │  edges)     │    │          │
│  │  └─────────────┘  └─────────────┘  └─────────────┘    │          │
│  │                                                      │          │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐    │          │
│  │  │ Cross-      │  │   Skill     │  │  Pattern    │    │          │
│  │  │ Encoder     │  │ Extraction  │  │  Learning   │    │          │
│  │  │ Reranker   │  │             │  │             │    │          │
│  │  └─────────────┘  └─────────────┘  └─────────────┘    │          │
│  │                                                      │          │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐    │          │
│  │  │Contrastive  │  │  Golden     │  │  Operator   │    │          │
│  │  │ Learning    │  │  Sample     │  │  Feedback   │    │          │
│  │  │             │  │  Testing   │  │             │    │          │
│  │  └─────────────┘  └─────────────┘  └─────────────┘    │          │
│  │                                                      │          │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐    │          │
│  │  │  Semantic   │  │ Procedural  │  │   Meta      │    │          │
│  │  │   Memory    │  │   Memory    │  │  Cognitive   │    │          │
│  │  │  (entities)│  │ (procedures)│  │ (self-know)  │    │          │
│  │  └─────────────┘  └─────────────┘  └─────────────┘    │          │
│  │                                                      │          │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐    │          │
│  │  │    Decay    │  │  Conflict   │  │  Staleness   │    │          │
│  │  │  (time-based│  │ Resolution  │  │  Detection  │    │          │
│  │  │  decay)     │  │             │  │             │    │          │
│  │  └─────────────┘  └─────────────┘  └─────────────┘    │          │
│  └──────────────────────────────────────────────────────────┘          │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
                           │ used by
                           ▼
              ┌────────────────────────┐
              │  swell-orchestrator   │
              └────────────────────────┘
```

**Key modules:**
- `lib.rs` — SqliteMemoryStore implementation with full MemoryStore trait
- `blocks.rs` — Memory block management (Project/User/Task)
- `recall.rs` — BM25 keyword search and temporal queries
- `triple_stream.rs` — Vector + BM25 + Graph retrieval with RRF
- `cross_encoder_rerank.rs` — BGE-reranker for result reranking
- `knowledge_graph.rs` — Property graph for code structure
- `skill_extraction.rs` — Skill extraction from trajectories
- `pattern_learning.rs` — Anti-pattern learning from feedback
- `contrastive_learning.rs` — Success/failure trajectory analysis
- `operator_feedback.rs` — CLAUDE.md/AGENTS.md parsing
- `golden_sample_testing.rs` — Procedure validation
- `semantic.rs` — Semantic entity and relation storage
- `procedural.rs` — Procedural memory with confidence scoring
- `meta_cognitive.rs` — Self-knowledge storage
- `decay.rs` — Time-based decay with type-specific rates
- `conflict_resolution.rs` — Memory conflict detection and resolution
- `staleness.rs` — Memory staleness detection
- `version_rollback.rs` — Version history and rollback
- `event_log.rs` — Append-only JSONL event log
- `evidence.rs` — Evidence pack for PR review

**Concurrency:** Uses `Arc<SqlitePool>` for connection pooling. All types are `Send + Sync`.

## Testing

```bash
# Run tests for swell-memory
cargo test -p swell-memory -- --test-threads=4

# Run with logging
RUST_LOG=debug cargo test -p swell-memory

# Run specific test module
cargo test -p swell-memory -- test_store_and_get --nocapture

# Run search tests
cargo test -p swell-memory -- test_search

# Run similarity check tests
cargo test -p swell-memory -- test_similarity

# Run staleness tests
cargo test -p swell-memory -- test_stale
```

**Test patterns:**
- Unit tests for store operations (store, get, update, delete)
- Search tests with various filters (query_text, block_types, labels, language, task_type)
- Repository isolation tests
- Similarity check tests (reject similar embeddings)
- Staleness detection and reinforcement tests
- Cross-encoder reranking tests

**Mock patterns:**
```rust
#[tokio::test]
async fn test_store_and_get() {
    let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

    let entry = MemoryEntry {
        id: Uuid::new_v4(),
        block_type: MemoryBlockType::Project,
        label: "test-project".to_string(),
        content: "Test content".to_string(),
        embedding: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        metadata: serde_json::json!({}),
        repository: "test-repo".to_string(),
        language: Some("rust".to_string()),
        task_type: None,
        last_reinforcement: None,
        is_stale: false,
        source_episode_id: None,
        evidence: None,
        provenance_context: None,
    };

    let id = store.store(entry.clone()).await.unwrap();
    let retrieved = store.get(id).await.unwrap();
    assert!(retrieved.is_some());
}
```

## Dependencies

```toml
# swell-memory/Cargo.toml
[dependencies]
swell-core = { path = "../swell-core" }
tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
anyhow.workspace = true
tracing.workspace = true
uuid.workspace = true
chrono.workspace = true
sqlx.workspace = true
async-trait.workspace = true

[dev-dependencies]
tempfile.workspace = true
```
