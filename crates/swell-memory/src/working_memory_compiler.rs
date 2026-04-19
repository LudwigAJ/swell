// working_memory_compiler.rs - Working Memory Compiler
//
// Assembles context from all memory layers into a 2,000-5,000 token budget
// per agent invocation. Prioritizes most relevant context within the budget constraint.
//
// Memory layers consumed:
// - Episodic (project/user/task blocks via memory store)
// - Semantic (entities and relations via semantic store)
// - Procedural (procedures with confidence via procedural store)
// - Skills (extracted skills via skill extraction)
// - Knowledge Graph (code structure via knowledge graph)
//
// The compiler:
// 1. Fetches entries from each layer filtered by repository scope
// 2. Scores entries by relevance (recency, confidence, block_type priority)
// 3. Packs entries into budget, highest priority first
// 4. Trims lower-priority entries to stay within token budget

use crate::procedural::ProceduralStore;
use crate::{
    skill_extraction::Skill, MemoryEntry, MemoryStore, SemanticEntityQuery, SemanticStore,
    SwellError,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

/// Token budget configuration for working memory compilation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingMemoryBudget {
    /// Minimum tokens for compiled context
    pub min_tokens: usize,
    /// Maximum tokens for compiled context
    pub max_tokens: usize,
    /// Tokens per word (approximate, used for estimation)
    pub tokens_per_word: f32,
}

impl Default for WorkingMemoryBudget {
    fn default() -> Self {
        // Default 2,000-5,000 token budget as per spec
        Self {
            min_tokens: 2000,
            max_tokens: 5000,
            tokens_per_word: 1.3, // ~1.3 tokens per word is a reasonable estimate
        }
    }
}

impl WorkingMemoryBudget {
    pub fn new(min_tokens: usize, max_tokens: usize) -> Self {
        Self {
            min_tokens,
            max_tokens,
            tokens_per_word: 1.3,
        }
    }
}

/// A layer source for working memory
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryLayer {
    /// Project/User/Task blocks
    Episodic,
    /// Semantic entities and relations
    Semantic,
    /// Procedural memories (strategies with confidence)
    Procedural,
    /// Extracted skills from trajectories
    Skills,
    /// Knowledge graph nodes and edges
    KnowledgeGraph,
}

impl MemoryLayer {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryLayer::Episodic => "episodic",
            MemoryLayer::Semantic => "semantic",
            MemoryLayer::Procedural => "procedural",
            MemoryLayer::Skills => "skills",
            MemoryLayer::KnowledgeGraph => "knowledge_graph",
        }
    }

    /// Priority score (higher = more important for working memory)
    pub fn priority_score(&self) -> f32 {
        match self {
            MemoryLayer::Episodic => 1.0,
            MemoryLayer::Semantic => 0.8,
            MemoryLayer::Procedural => 0.9,
            MemoryLayer::Skills => 0.7,
            MemoryLayer::KnowledgeGraph => 0.6,
        }
    }
}

/// A scored memory entry ready for compilation
#[derive(Debug, Clone)]
pub struct ScoredEntry {
    pub entry: MemoryEntry,
    pub score: f32,
    pub layer: MemoryLayer,
}

impl ScoredEntry {
    pub fn new(entry: MemoryEntry, score: f32, layer: MemoryLayer) -> Self {
        Self {
            entry,
            score,
            layer,
        }
    }
}

/// Working memory compilation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingMemory {
    /// The assembled context content
    pub content: String,
    /// Estimated token count
    pub token_count: usize,
    /// Number of entries included
    pub entry_count: usize,
    /// Layers that contributed to this working memory
    pub layers_used: Vec<MemoryLayer>,
    /// Whether output was trimmed to fit budget
    pub was_trimmed: bool,
    /// Entries that were trimmed due to budget
    pub trimmed_entries: Vec<String>,
}

impl WorkingMemory {
    /// Check if token count is within budget range
    pub fn is_within_budget(&self, budget: &WorkingMemoryBudget) -> bool {
        self.token_count >= budget.min_tokens && self.token_count <= budget.max_tokens
    }

    /// Create an empty working memory
    pub fn empty() -> Self {
        Self {
            content: String::new(),
            token_count: 0,
            entry_count: 0,
            layers_used: Vec::new(),
            was_trimmed: false,
            trimmed_entries: Vec::new(),
        }
    }

    /// Create a working memory from content
    pub fn from_content(content: String, layers: Vec<MemoryLayer>) -> Self {
        let token_count = Self::estimate_tokens(&content);
        Self {
            content,
            token_count,
            entry_count: 0,
            layers_used: layers,
            was_trimmed: false,
            trimmed_entries: Vec::new(),
        }
    }

    /// Estimate token count from text (word_count * 1.3)
    pub fn estimate_tokens(text: &str) -> usize {
        let word_count = text.split_whitespace().count();
        (word_count as f32 * 1.3) as usize
    }
}

/// Memory layer contribution tracking
#[derive(Debug, Clone, Default)]
pub struct LayerContribution {
    pub episodic: usize,
    pub semantic: usize,
    pub procedural: usize,
    pub skills: usize,
    pub knowledge_graph: usize,
}

impl LayerContribution {
    pub fn add(&mut self, layer: MemoryLayer, tokens: usize) {
        match layer {
            MemoryLayer::Episodic => self.episodic += tokens,
            MemoryLayer::Semantic => self.semantic += tokens,
            MemoryLayer::Procedural => self.procedural += tokens,
            MemoryLayer::Skills => self.skills += tokens,
            MemoryLayer::KnowledgeGraph => self.knowledge_graph += tokens,
        }
    }

    pub fn layer_count(&self) -> usize {
        let mut count = 0;
        if self.episodic > 0 {
            count += 1;
        }
        if self.semantic > 0 {
            count += 1;
        }
        if self.procedural > 0 {
            count += 1;
        }
        if self.skills > 0 {
            count += 1;
        }
        if self.knowledge_graph > 0 {
            count += 1;
        }
        count
    }
}

/// Configuration for working memory compiler
#[derive(Debug, Clone)]
pub struct WorkingMemoryCompilerConfig {
    pub budget: WorkingMemoryBudget,
    pub include_episodic: bool,
    pub include_semantic: bool,
    pub include_procedural: bool,
    pub include_skills: bool,
    pub include_knowledge_graph: bool,
    pub max_entries_per_layer: usize,
    pub recency_boost_days: i64,
}

impl Default for WorkingMemoryCompilerConfig {
    fn default() -> Self {
        Self {
            budget: WorkingMemoryBudget::default(),
            include_episodic: true,
            include_semantic: true,
            include_procedural: true,
            include_skills: true,
            include_knowledge_graph: true,
            max_entries_per_layer: 20,
            recency_boost_days: 7,
        }
    }
}

/// Working Memory Compiler
///
/// Assembles context from all memory layers into a constrained token budget.
pub struct WorkingMemoryCompiler {
    config: WorkingMemoryCompilerConfig,
}

impl WorkingMemoryCompiler {
    pub fn new(config: WorkingMemoryCompilerConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config() -> Self {
        Self::new(WorkingMemoryCompilerConfig::default())
    }

    /// Compile working memory from all available layers
    pub async fn compile(
        &self,
        memory_store: Arc<dyn MemoryStore>,
        semantic_store: Option<Arc<dyn SemanticStore>>,
        procedural_store: Option<Arc<dyn ProceduralStore>>,
        skills: Option<Vec<Skill>>,
        repository: &str,
        task_id: Option<Uuid>,
    ) -> Result<WorkingMemory, SwellError> {
        let mut scored_entries: Vec<ScoredEntry> = Vec::new();
        let mut contributions = LayerContribution::default();

        // 1. Fetch from episodic layer (project/user/task blocks via MemoryStore)
        if self.config.include_episodic {
            let entries = self
                .fetch_episodic_entries(&*memory_store, repository, task_id)
                .await?;
            for entry in entries {
                let score = self.score_episodic_entry(&entry);
                let tokens = self.estimate_entry_tokens(&entry);
                contributions.add(MemoryLayer::Episodic, tokens);
                scored_entries.push(ScoredEntry::new(entry, score, MemoryLayer::Episodic));
            }
        }

        // 2. Fetch from semantic layer
        if self.config.include_semantic {
            if let Some(ref store) = semantic_store {
                let entities = self.fetch_semantic_entries(store.as_ref()).await?;
                for entity in entities {
                    let score = self.score_semantic_entry(&entity);
                    let tokens = self.estimate_entity_tokens(&entity);
                    contributions.add(MemoryLayer::Semantic, tokens);
                    // Convert semantic entity to MemoryEntry for unified handling
                    let entry = self.entity_to_memory_entry(entity);
                    scored_entries.push(ScoredEntry::new(entry, score, MemoryLayer::Semantic));
                }
            }
        }

        // 3. Fetch from procedural layer
        if self.config.include_procedural {
            if let Some(ref store) = procedural_store {
                let procedures = self.fetch_procedural_entries(store.as_ref()).await?;
                for procedure in procedures {
                    let score = self.score_procedural_entry(&procedure);
                    let tokens = self.estimate_procedure_tokens(&procedure);
                    contributions.add(MemoryLayer::Procedural, tokens);
                    let entry = self.procedure_to_memory_entry(procedure);
                    scored_entries.push(ScoredEntry::new(entry, score, MemoryLayer::Procedural));
                }
            }
        }

        // 4. Add skills
        if self.config.include_skills {
            if let Some(skills) = skills {
                for skill in skills {
                    let score = self.score_skill(&skill);
                    let tokens = self.estimate_skill_tokens(&skill);
                    contributions.add(MemoryLayer::Skills, tokens);
                    let entry = self.skill_to_memory_entry(skill);
                    scored_entries.push(ScoredEntry::new(entry, score, MemoryLayer::Skills));
                }
            }
        }

        // 5. Sort by score (highest first) and pack into budget
        scored_entries.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut content_parts: Vec<String> = Vec::new();
        let mut used_layers: Vec<MemoryLayer> = Vec::new();
        let mut trimmed_entries: Vec<String> = Vec::new();
        let mut total_tokens = 0;
        let max_tokens = self.config.budget.max_tokens;

        for scored in scored_entries {
            let entry_tokens = self.estimate_entry_tokens(&scored.entry);

            if total_tokens + entry_tokens > max_tokens {
                // Try to add partial content if we haven't reached min_tokens yet
                if total_tokens < self.config.budget.min_tokens {
                    let remaining_budget = max_tokens - total_tokens;
                    if entry_tokens > remaining_budget {
                        // Add as much as we can
                        let trimmed_content =
                            self.trim_entry_to_tokens(&scored.entry, remaining_budget);
                        let trimmed_tokens = WorkingMemory::estimate_tokens(&trimmed_content);
                        content_parts.push(trimmed_content);
                        total_tokens += trimmed_tokens;
                        trimmed_entries.push(scored.entry.id.to_string());
                        if !used_layers.contains(&scored.layer) {
                            used_layers.push(scored.layer);
                        }
                    }
                } else {
                    trimmed_entries.push(scored.entry.id.to_string());
                }
                continue;
            }

            content_parts.push(scored.entry.content.clone());
            total_tokens += entry_tokens;
            if !used_layers.contains(&scored.layer) {
                used_layers.push(scored.layer);
            }
        }

        let content = content_parts.join("\n\n---\n\n");
        let was_trimmed =
            !trimmed_entries.is_empty() || total_tokens > self.config.budget.min_tokens;

        Ok(WorkingMemory {
            content,
            token_count: total_tokens,
            entry_count: content_parts.len(),
            layers_used: used_layers,
            was_trimmed,
            trimmed_entries,
        })
    }

    /// Fetch episodic entries (project/user/task blocks) from memory store
    async fn fetch_episodic_entries(
        &self,
        store: &dyn MemoryStore,
        repository: &str,
        task_id: Option<Uuid>,
    ) -> Result<Vec<MemoryEntry>, SwellError> {
        let mut all_entries: Vec<MemoryEntry> = Vec::new();

        // Fetch project blocks
        let project_entries = store
            .get_by_type(swell_core::MemoryBlockType::Project, repository.to_string())
            .await?;
        all_entries.extend(project_entries);

        // Fetch user blocks
        let user_entries = store
            .get_by_type(swell_core::MemoryBlockType::User, repository.to_string())
            .await?;
        all_entries.extend(user_entries);

        // Fetch task block if task_id provided
        if let Some(tid) = task_id {
            let task_entries = store
                .get_by_label(format!("task:{}", tid), repository.to_string())
                .await?;
            all_entries.extend(task_entries);
        }

        // Limit entries per layer
        all_entries.truncate(self.config.max_entries_per_layer);

        Ok(all_entries)
    }

    /// Fetch semantic entries from semantic store
    async fn fetch_semantic_entries(
        &self,
        store: &dyn SemanticStore,
    ) -> Result<Vec<crate::SemanticEntity>, SwellError> {
        let query = SemanticEntityQuery {
            entity_types: None,
            name_contains: None,
            limit: self.config.max_entries_per_layer,
            offset: 0,
        };

        store.query_entities(query).await
    }

    /// Fetch procedural entries from procedural store
    async fn fetch_procedural_entries(
        &self,
        store: &dyn ProceduralStore,
    ) -> Result<Vec<crate::Procedure>, SwellError> {
        let query = crate::ProcedureQuery {
            keywords: None,
            min_confidence: Some(0.3), // Only include procedures with confidence >= 0.3
            min_uses: None,
            limit: self.config.max_entries_per_layer,
            offset: 0,
        };

        let results = store.find_by_context(query).await?;
        Ok(results.into_iter().map(|r| r.procedure).collect())
    }

    /// Score an episodic entry based on recency and block type
    fn score_episodic_entry(&self, entry: &MemoryEntry) -> f32 {
        let mut score = 0.5; // Base score

        // Higher score for more recent updates
        let now = Utc::now();
        let age_days = (now - entry.updated_at).num_days() as f32;
        let recency_factor = (-age_days / 30.0).exp().min(1.0);
        score += recency_factor * 0.3;

        // Block type priority
        match entry.block_type {
            swell_core::MemoryBlockType::Task => score += 0.15,
            swell_core::MemoryBlockType::Project => score += 0.1,
            swell_core::MemoryBlockType::User => score += 0.05,
            swell_core::MemoryBlockType::Skill => score += 0.08,
            swell_core::MemoryBlockType::Convention => score += 0.06,
        }

        // Staleness penalty
        if entry.is_stale {
            score *= 0.5;
        }

        score
    }

    /// Score a semantic entity
    fn score_semantic_entry(&self, entity: &crate::SemanticEntity) -> f32 {
        let mut score = 0.5;

        // Entity type priority
        match entity.entity_type {
            crate::SemanticEntityType::Task
            | crate::SemanticEntityType::Requirement
            | crate::SemanticEntityType::Skill => score += 0.2,
            crate::SemanticEntityType::Convention
            | crate::SemanticEntityType::Configuration
            | crate::SemanticEntityType::Dependency => score += 0.15,
            _ => score += 0.05,
        }

        // Recency boost
        let now = Utc::now();
        let age_days = (now - entity.updated_at).num_days() as f32;
        let recency_factor = (-age_days / 30.0).exp().min(1.0);
        score += recency_factor * 0.2;

        score
    }

    /// Score a procedural entry
    fn score_procedural_entry(&self, procedure: &crate::Procedure) -> f32 {
        let mut score = 0.5;

        // Beta posterior mean as confidence
        let confidence = procedure.effectiveness.mean() as f32;
        score += confidence * 0.4;

        // Usage count boost (up to 0.1)
        let usage_boost = (procedure.usage_count as f32 / 100.0).min(0.1);
        score += usage_boost;

        score
    }

    /// Score a skill entry
    fn score_skill(&self, skill: &Skill) -> f32 {
        let mut score = 0.5;

        // Confidence contributes (skill.confidence is f64)
        score += (skill.confidence as f32) * 0.4;

        score
    }

    /// Estimate tokens for a memory entry
    fn estimate_entry_tokens(&self, entry: &MemoryEntry) -> usize {
        (entry.content.split_whitespace().count() as f32 * self.config.budget.tokens_per_word)
            as usize
    }

    /// Estimate tokens for a semantic entity
    fn estimate_entity_tokens(&self, entity: &crate::SemanticEntity) -> usize {
        let text = format!("{}: {}", entity.name, entity.name);
        (text.split_whitespace().count() as f32 * self.config.budget.tokens_per_word) as usize
    }

    /// Estimate tokens for a procedure
    fn estimate_procedure_tokens(&self, procedure: &crate::Procedure) -> usize {
        let text = format!("{}: {}", procedure.name, procedure.description);
        (text.split_whitespace().count() as f32 * self.config.budget.tokens_per_word) as usize
    }

    /// Estimate tokens for a skill
    fn estimate_skill_tokens(&self, skill: &Skill) -> usize {
        let text = format!("{}: {}", skill.name, skill.description);
        (text.split_whitespace().count() as f32 * self.config.budget.tokens_per_word) as usize
    }

    /// Convert semantic entity to MemoryEntry
    fn entity_to_memory_entry(&self, entity: crate::SemanticEntity) -> MemoryEntry {
        let properties_json = entity.properties;
        let description = properties_json
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        MemoryEntry {
            id: entity.id,
            block_type: swell_core::MemoryBlockType::Project,
            label: format!("semantic:{}", entity.name),
            content: format!("{}: {}", entity.name, description),
            embedding: None,
            created_at: entity.created_at,
            updated_at: entity.updated_at,
            metadata: serde_json::json!({
                "entity_type": entity.entity_type.as_str(),
            }),
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
        }
    }

    /// Convert procedure to MemoryEntry
    fn procedure_to_memory_entry(&self, procedure: crate::Procedure) -> MemoryEntry {
        MemoryEntry {
            id: procedure.id,
            block_type: swell_core::MemoryBlockType::Project,
            label: format!("procedure:{}", procedure.name),
            content: format!(
                "{}: {}\n\nSteps:\n{}",
                procedure.name,
                procedure.description,
                procedure
                    .steps
                    .iter()
                    .enumerate()
                    .map(|(i, s)| format!("{}. {}", i + 1, s.description))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            embedding: None,
            created_at: procedure.created_at,
            updated_at: procedure.updated_at,
            metadata: serde_json::json!({
                "effectiveness_alpha": procedure.effectiveness.alpha,
                "effectiveness_beta": procedure.effectiveness.beta,
                "usage_count": procedure.usage_count,
            }),
            repository: String::new(),
            org: String::new(),
            workspace: String::new(),
            language: None,
            framework: None,
            environment: None,
            task_type: None,
            session_id: None,
            last_reinforcement: procedure.last_used,
            is_stale: false,
            source_episode_id: None,
            evidence: None,
            provenance_context: None,
        }
    }

    /// Convert skill to MemoryEntry
    fn skill_to_memory_entry(&self, skill: Skill) -> MemoryEntry {
        MemoryEntry {
            id: skill.id,
            block_type: swell_core::MemoryBlockType::Project,
            label: format!("skill:{}", skill.name),
            content: format!(
                "# {}\n\n{}\n\nTask Pattern: {}\n\nSteps:\n{}\n\nTools Used: {}",
                skill.name,
                skill.description,
                skill.task_pattern,
                skill
                    .steps
                    .iter()
                    .map(|s| s.description.clone())
                    .collect::<Vec<_>>()
                    .join("\n"),
                skill.tools_used.join(", ")
            ),
            embedding: None,
            created_at: skill.created_at,
            updated_at: skill.created_at,
            metadata: serde_json::json!({
                "confidence": skill.confidence,
                "source_task_id": skill.source_task_id.to_string(),
            }),
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
        }
    }

    /// Trim entry content to fit within token budget
    fn trim_entry_to_tokens(&self, entry: &MemoryEntry, max_tokens: usize) -> String {
        let max_words = (max_tokens as f32 / self.config.budget.tokens_per_word) as usize;
        let words: Vec<&str> = entry.content.split_whitespace().collect();

        if words.len() <= max_words {
            entry.content.clone()
        } else {
            words[..max_words].join(" ")
        }
    }
}

/// Default working memory compiler instance
pub fn create_compiler(config: WorkingMemoryCompilerConfig) -> WorkingMemoryCompiler {
    WorkingMemoryCompiler::new(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill_extraction::SkillStep;
    use crate::SqliteMemoryStore;

    #[test]
    fn test_budget_default() {
        let budget = WorkingMemoryBudget::default();
        assert_eq!(budget.min_tokens, 2000);
        assert_eq!(budget.max_tokens, 5000);
    }

    #[test]
    fn test_memory_layer_priority() {
        assert!(
            MemoryLayer::Episodic.priority_score() >= MemoryLayer::KnowledgeGraph.priority_score()
        );
        assert!(MemoryLayer::Procedural.priority_score() >= MemoryLayer::Skills.priority_score());
    }

    #[test]
    fn test_working_memory_empty() {
        let wm = WorkingMemory::empty();
        assert!(wm.content.is_empty());
        assert_eq!(wm.token_count, 0);
        assert!(!wm.was_trimmed);
    }

    #[test]
    fn test_working_memory_from_content() {
        let content = "Test content".to_string();
        let wm = WorkingMemory::from_content(content.clone(), vec![MemoryLayer::Episodic]);

        assert_eq!(wm.content, content);
        assert_eq!(wm.entry_count, 0); // from_content doesn't set entry_count
        assert_eq!(wm.layers_used, vec![MemoryLayer::Episodic]);
        assert!(!wm.was_trimmed);
    }

    #[test]
    fn test_working_memory_is_within_budget() {
        let budget = WorkingMemoryBudget::default();

        let mut wm = WorkingMemory::empty();
        wm.token_count = 3000;
        assert!(wm.is_within_budget(&budget));

        wm.token_count = 1500;
        assert!(!wm.is_within_budget(&budget)); // Below min

        wm.token_count = 6000;
        assert!(!wm.is_within_budget(&budget)); // Above max
    }

    #[test]
    fn test_layer_contribution() {
        let mut contrib = LayerContribution::default();
        contrib.add(MemoryLayer::Episodic, 100);
        contrib.add(MemoryLayer::Semantic, 200);
        contrib.add(MemoryLayer::Procedural, 150);

        assert_eq!(contrib.layer_count(), 3);
        assert_eq!(contrib.episodic, 100);
        assert_eq!(contrib.semantic, 200);
        assert_eq!(contrib.procedural, 150);
    }

    #[test]
    fn test_estimate_tokens() {
        let text = "one two three four five";
        let tokens = WorkingMemory::estimate_tokens(text);
        // 5 words * 1.3 = ~6.5, floored to 6
        assert!(tokens >= 6 && tokens <= 7);
    }

    #[tokio::test]
    async fn test_compiler_empty_store() {
        use crate::SqliteMemoryStore;

        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();
        let compiler = WorkingMemoryCompiler::with_default_config();

        let result = compiler
            .compile(Arc::new(store), None, None, None, "test-repo", None)
            .await
            .unwrap();

        // Empty store should produce empty working memory
        assert!(result.content.is_empty());
        assert_eq!(result.token_count, 0);
        assert!(result.layers_used.is_empty());
    }

    #[tokio::test]
    async fn test_compiler_with_entries() {
        use crate::{MemoryBlockType, MemoryEntry};
        use uuid::Uuid;

        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        // Store a project block
        let entry = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "test-project".to_string(),
            content: "This is a test project with substantial content that should be included in the working memory assembly.".to_string(),
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

            last_reinforcement: Some(chrono::Utc::now()),
            is_stale: false,
            source_episode_id: None,
            evidence: None,
            provenance_context: None,
        };

        store.store(entry.clone()).await.unwrap();

        let compiler = WorkingMemoryCompiler::with_default_config();

        let result = compiler
            .compile(Arc::new(store), None, None, None, "test-repo", None)
            .await
            .unwrap();

        // Should include content from the entry
        assert!(!result.content.is_empty());
        assert!(result.token_count > 0);
        assert!(result.layers_used.contains(&MemoryLayer::Episodic));
    }

    #[tokio::test]
    async fn test_compiler_multiple_layers() {
        use crate::{MemoryBlockType, MemoryEntry};
        use uuid::Uuid;

        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        // Store multiple entries of different types
        let project_entry = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "project".to_string(),
            content:
                "Project architecture details with important information about the system design."
                    .to_string(),
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

            last_reinforcement: Some(chrono::Utc::now()),
            is_stale: false,
            source_episode_id: None,
            evidence: None,
            provenance_context: None,
        };

        let user_entry = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::User,
            label: "user:default".to_string(),
            content: "User preferences for code style and tool usage patterns.".to_string(),
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

            last_reinforcement: Some(chrono::Utc::now()),
            is_stale: false,
            source_episode_id: None,
            evidence: None,
            provenance_context: None,
        };

        store.store(project_entry.clone()).await.unwrap();
        store.store(user_entry.clone()).await.unwrap();

        let compiler = WorkingMemoryCompiler::with_default_config();

        let result = compiler
            .compile(Arc::new(store), None, None, None, "test-repo", None)
            .await
            .unwrap();

        // Should have entries from multiple layers (both Project and User blocks are Episodic)
        assert!(!result.content.is_empty());
        assert!(result.token_count > 0);
        // Note: Both are in Episodic layer, so layer count is 1 but entry count is 2
        assert_eq!(result.entry_count, 2);
    }

    #[tokio::test]
    async fn test_compiler_token_budget() {
        use crate::{MemoryBlockType, MemoryEntry};
        use uuid::Uuid;

        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        // Add entries from multiple memory layers to verify multi-layer content
        // Layer 1: Episodic (Project blocks) - substantial content to reach min budget
        for i in 0..10 {
            let entry = MemoryEntry {
                id: Uuid::new_v4(),
                block_type: MemoryBlockType::Project,
                label: format!("project-{}", i),
                content: format!(
                    "Project {} contains detailed architectural decisions and implementation notes for the working memory system. This includes information about token budgeting, context assembly, and prioritization strategies. The system must efficiently pack relevant context within the 2000-5000 token budget constraint while ensuring maximum utility of the included information. Key considerations include recency scoring, block type priority, and relevance ranking to ensure the most important memories are included first. The working memory compiler interfaces with multiple memory layers including episodic storage for project and task blocks, semantic storage for entity recognition, procedural storage for learned patterns, and skill extraction for reusable agent behaviors. Each layer contributes different types of context that inform the autonomous coding engine during task execution. The compilation process prioritizes entries based on recency and relevance scores calculated from temporal decay functions and block type weights. Content that exceeds available budget is intelligently trimmed while preserving the most critical information for agent decision-making.",
                    i
                ),
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

                last_reinforcement: Some(chrono::Utc::now()),
                is_stale: false,
                source_episode_id: None,
                evidence: None,
                provenance_context: None,
            };
            store.store(entry).await.unwrap();
        }

        // Layer 2: Episodic (User blocks) - different block type but still Episodic layer
        for i in 0..10 {
            let entry = MemoryEntry {
                id: Uuid::new_v4(),
                block_type: MemoryBlockType::User,
                label: format!("user-pref-{}", i),
                content: format!(
                    "User preference {} defines coding standards and tool usage patterns for this repository. These preferences guide the autonomous coding engine in making consistent decisions about code style, naming conventions, and tool selection. The preferences are learned over time through reinforcement and operator feedback, forming an important part of the contextual memory that informs agent behavior. User blocks store individual settings like preferred linters, formatting rules, test frameworks, and deployment practices. The system tracks preference usage frequency and effectiveness through the meta-cognitive layer, allowing it to suggest optimizations and adapt to changing development workflows. When compiling working memory, user preferences receive high priority due to their direct impact on code quality and consistency. Preference conflicts are resolved through conflict resolution algorithms that consider recency, usage count, and explicit operator overrides.",
                    i
                ),
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

                last_reinforcement: Some(chrono::Utc::now()),
                is_stale: false,
                source_episode_id: None,
                evidence: None,
                provenance_context: None,
            };
            store.store(entry).await.unwrap();
        }

        // Layer 3: Add a Task block - also Episodic but distinct
        for i in 0..5 {
            let entry = MemoryEntry {
                id: Uuid::new_v4(),
                block_type: MemoryBlockType::Task,
                label: format!("task-{}", i),
                content: format!(
                    "Task {} represents a specific coding objective with detailed requirements and acceptance criteria. The task context includes implementation hints, related files, and success metrics that guide the agent toward completing the objective effectively. Task memories form a critical part of episodic recall and help maintain continuity across interrupted or multi-session work. Each task block tracks progress through reinforcement signals, allowing the system to learn which approaches lead to successful completion. Failed attempts are analyzed through contrastive learning to identify failure patterns and recovery strategies. Task metadata includes skill usage patterns, tool invocation sequences, and validation gate results that inform future similar tasks.",
                    i
                ),
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

                last_reinforcement: Some(chrono::Utc::now()),
                is_stale: false,
                source_episode_id: None,
                evidence: None,
                provenance_context: None,
            };
            store.store(entry).await.unwrap();
        }

        // Layer 2: Skills - provides a different memory layer than Episodic
        let skills = vec![
            Skill {
                id: Uuid::new_v4(),
                name: "Rust Error Handling Pattern".to_string(),
                description: "Standard pattern for implementing error handling in Rust using thiserror and anyhow crates. This includes proper error type definition, context addition, and error propagation patterns.".to_string(),
                version: "1.0.0".to_string(),
                task_pattern: "Implementing error types and handling in Rust".to_string(),
                steps: vec![
                    SkillStep {
                        order: 0,
                        description: "Define error enum with thiserror derive macro for domain errors".to_string(),
                        affected_file_patterns: vec!["**/error.rs".to_string()],
                        tool_sequence: vec!["file::read".to_string(), "file::edit".to_string()],
                        validation_check: Some("cargo check passes".to_string()),
                    },
                    SkillStep {
                        order: 1,
                        description: "Use anyhow::Context for operations that need rich error messages".to_string(),
                        affected_file_patterns: vec!["**/*.rs".to_string()],
                        tool_sequence: vec!["file::edit".to_string()],
                        validation_check: Some("cargo build succeeds".to_string()),
                    },
                ],
                tools_used: vec!["file::read".to_string(), "file::edit".to_string(), "shell".to_string()],
                conventions: vec!["Use thiserror for domain errors".to_string(), "Use anyhow for context-rich errors".to_string()],
                confidence: 0.85,
                source_task_id: TaskId::new(),
                created_at: chrono::Utc::now(),
                metadata: serde_json::json!({}),
            },
            Skill {
                id: Uuid::new_v4(),
                name: "Async Test Writing".to_string(),
                description: "Pattern for writing async tests in Rust using tokio::test. Covers async function testing, shared state management, and proper test isolation patterns for concurrent test execution.".to_string(),
                version: "1.0.0".to_string(),
                task_pattern: "Writing async unit tests with Tokio".to_string(),
                steps: vec![
                    SkillStep {
                        order: 0,
                        description: "Use #[tokio::test] for async test functions".to_string(),
                        affected_file_patterns: vec!["**/*_test.rs".to_string()],
                        tool_sequence: vec!["file::write".to_string()],
                        validation_check: Some("cargo test passes".to_string()),
                    },
                ],
                tools_used: vec!["file::write".to_string(), "shell".to_string()],
                conventions: vec!["Always use #[tokio::test] for async tests".to_string()],
                confidence: 0.9,
                source_task_id: TaskId::new(),
                created_at: chrono::Utc::now(),
                metadata: serde_json::json!({}),
            },
        ];

        let config = WorkingMemoryCompilerConfig {
            budget: WorkingMemoryBudget::new(2000, 5000),
            ..Default::default()
        };
        let compiler = WorkingMemoryCompiler::new(config);

        let result = compiler
            .compile(Arc::new(store), None, None, Some(skills), "test-repo", None)
            .await
            .unwrap();

        // Token count should be within budget (2000-5000 as per VAL-MEM-005)
        assert!(
            result.token_count >= 2000,
            "Token count {} should be >= 2000 (minimum token budget)",
            result.token_count
        );
        assert!(
            result.token_count <= 5000,
            "Token count {} should be <= 5000 (maximum token budget)",
            result.token_count
        );

        // Verify content comes from >= 2 different memory layers
        assert!(
            result.layers_used.len() >= 2,
            "Content should come from >= 2 different memory layers, but got {:?}",
            result.layers_used
        );
    }
}
