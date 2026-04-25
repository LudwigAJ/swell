//! Context Pipeline Module
//!
//! Provides tiered context assembly with token budget enforcement and auto-compaction.
//!
//! # Tiers
//!
//! - **Tier 1 (Highest)**: Active file being edited
//! - **Tier 2**: Open/recent files
//! - **Tier 3**: Vector search results (semantic relevance)
//! - **Tier 4**: Graph-expanded context (dependencies, callers)
//! - **Tier 5 (Lowest)**: Conversation history (recall)
//!
//! # Features
//!
//! - Token budget enforcement with truncation
//! - Tier-based scoring and prioritization
//! - Auto-compaction when exceeding threshold
//! - Configurable limits per tier

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ============================================================================
// Tier Configuration and Context Types
// ============================================================================

/// Configuration for context pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPipelineConfig {
    /// Maximum tokens in the context window
    pub max_tokens: usize,
    /// Warning threshold (0.0 to 1.0) - triggers warning at this percentage
    pub warning_threshold: f64,
    /// Condensation threshold (0.0 to 1.0) - triggers auto-compaction
    pub condensation_threshold: f64,
    /// Maximum items per tier
    pub max_items_per_tier: usize,
    /// Whether to enable auto-compaction
    pub auto_compaction_enabled: bool,
    /// Weights for each tier (higher = more important)
    pub tier_weights: HashMap<ContextTier, u32>,
}

impl Default for ContextPipelineConfig {
    fn default() -> Self {
        let mut tier_weights = HashMap::new();
        tier_weights.insert(ContextTier::ActiveFile, 100);
        tier_weights.insert(ContextTier::OpenRecent, 80);
        tier_weights.insert(ContextTier::VectorSearch, 60);
        tier_weights.insert(ContextTier::GraphExpanded, 40);
        tier_weights.insert(ContextTier::ConversationHistory, 20);

        Self {
            max_tokens: 100_000,
            warning_threshold: 0.65,
            condensation_threshold: 0.75,
            max_items_per_tier: 50,
            auto_compaction_enabled: true,
            tier_weights,
        }
    }
}

/// Context tiers ordered by priority (highest to lowest)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum ContextTier {
    /// Active file being edited - highest priority
    ActiveFile = 0,
    /// Open/recent files
    OpenRecent = 1,
    /// Vector search results (semantic relevance)
    VectorSearch = 2,
    /// Graph-expanded context (dependencies, callers)
    GraphExpanded = 3,
    /// Conversation history (recall)
    ConversationHistory = 4,
}

impl ContextTier {
    /// Get the priority score for this tier (higher = more important)
    pub fn priority(&self, config: &ContextPipelineConfig) -> u32 {
        config.tier_weights.get(self).copied().unwrap_or(50)
    }

    /// Check if this tier needs condensation before lower tiers
    pub fn needs_preservation_before(&self, other: &ContextTier) -> bool {
        *self < *other
    }
}

/// A context item with metadata for scoring and truncation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineContextItem {
    /// Unique identifier for this item
    pub id: String,
    /// The actual content text
    pub content: String,
    /// Estimated token count
    pub tokens: usize,
    /// Source tier
    pub tier: ContextTier,
    /// Relevance score (0.0 to 1.0)
    pub relevance_score: f32,
    /// Priority within tier (higher = more important)
    pub priority: u32,
    /// Source identifier (file path, entity id, etc.)
    pub source_id: Option<String>,
    /// Whether this item has been modified recently
    pub is_recent: bool,
}

impl PipelineContextItem {
    /// Create a new context item with auto-estimated tokens
    pub fn new(content: String, tier: ContextTier, relevance_score: f32) -> Self {
        let tokens = estimate_tokens(&content);
        Self {
            id: Uuid::new_v4().to_string(),
            content,
            tokens,
            tier,
            relevance_score,
            priority: 50,
            source_id: None,
            is_recent: false,
        }
    }

    /// Create a new context item with explicit token count
    pub fn with_explicit_tokens(
        content: String,
        tier: ContextTier,
        relevance_score: f32,
        tokens: usize,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            content,
            tokens,
            tier,
            relevance_score,
            priority: 50,
            source_id: None,
            is_recent: false,
        }
    }

    /// Calculate combined priority score for sorting
    pub fn priority_score(&self, config: &ContextPipelineConfig) -> u64 {
        let tier_priority = self.tier.priority(config) as u64;
        let item_priority = self.priority as u64;
        let relevance = (self.relevance_score * 100.0) as u64;
        let recent_bonus = if self.is_recent { 10 } else { 0 };

        // Higher tier priority, higher item priority, higher relevance, recent bonus
        (tier_priority * 10000) + (item_priority * 100) + relevance + recent_bonus
    }

    /// Set the source ID
    pub fn with_source_id(mut self, source_id: String) -> Self {
        self.source_id = Some(source_id);
        self
    }

    /// Set the priority
    pub fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }

    /// Set the recent flag
    pub fn with_recent(mut self, is_recent: bool) -> Self {
        self.is_recent = is_recent;
        self
    }
}

/// Estimated token count using simple word-based approximation
fn estimate_tokens(content: &str) -> usize {
    // Rough approximation: ~4 characters per token on average
    (content.len() / 4).max(1)
}

/// Result of context pipeline assembly
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPipelineResult {
    /// All assembled context items
    pub items: Vec<PipelineContextItem>,
    /// Total estimated tokens
    pub total_tokens: usize,
    /// Original tokens before truncation (if any)
    pub original_tokens: Option<usize>,
    /// Tokens remaining after budget enforcement
    pub budget_remaining: usize,
    /// Whether truncation was applied
    pub was_truncated: bool,
    /// Condensation level when assembly completed
    pub condensation_level: CondensationLevel,
    /// Items removed due to truncation
    pub removed_items: Vec<String>,
}

// ============================================================================
// Condensation Level
// ============================================================================

/// Level of context condensation needed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CondensationLevel {
    /// Context is within acceptable limits
    Ok,
    /// Context is approaching limit (warning threshold)
    Warning,
    /// Context must be condensed (condensation threshold)
    MustCondense,
}

// ============================================================================
// Context Assembler (Legacy Support)
// ============================================================================

/// Assembler that combines context from multiple sources
#[derive(Debug, Clone)]
pub struct ContextAssembler {
    config: ContextPipelineConfig,
}

impl Default for ContextAssembler {
    fn default() -> Self {
        Self::new(ContextPipelineConfig::default())
    }
}

impl ContextAssembler {
    /// Create a new context assembler with the given config
    pub fn new(config: ContextPipelineConfig) -> Self {
        Self { config }
    }

    /// Create a context assembler with default config
    pub fn with_default_config() -> Self {
        Self::new(ContextPipelineConfig::default())
    }

    /// Assemble context from items, applying budget enforcement
    pub fn assemble(&self, mut items: Vec<PipelineContextItem>) -> ContextPipelineResult {
        let original_tokens: usize = items.iter().map(|i| i.tokens).sum();

        // Sort by priority score
        items.sort_by(|a, b| {
            b.priority_score(&self.config)
                .cmp(&a.priority_score(&self.config))
        });

        // Apply tier-based filtering first
        let tier_filtered = self.filter_by_tier(items);

        // Calculate total and check against budget
        let total_tokens: usize = tier_filtered.iter().map(|i| i.tokens).sum();
        let condensation_level = self.calculate_condensation_level(total_tokens);

        // If over budget and auto-compaction enabled, condense
        let (final_items, was_truncated, removed_items) =
            if total_tokens > self.config.max_tokens && self.config.auto_compaction_enabled {
                self.truncate_to_budget(tier_filtered)
            } else {
                (tier_filtered, false, Vec::new())
            };

        // Recalculate total after truncation
        let final_tokens: usize = final_items.iter().map(|i| i.tokens).sum();
        let budget_remaining = self.config.max_tokens.saturating_sub(final_tokens);

        ContextPipelineResult {
            items: final_items,
            total_tokens: final_tokens,
            original_tokens: if was_truncated {
                Some(original_tokens)
            } else {
                None
            },
            budget_remaining,
            was_truncated,
            condensation_level,
            removed_items,
        }
    }

    /// Filter items by tier, enforcing max per tier
    fn filter_by_tier(&self, items: Vec<PipelineContextItem>) -> Vec<PipelineContextItem> {
        let mut tier_counts: HashMap<ContextTier, usize> = HashMap::new();
        let mut result = Vec::new();

        for item in items {
            let count = tier_counts.entry(item.tier).or_insert(0);
            if *count < self.config.max_items_per_tier {
                result.push(item);
                *count += 1;
            }
        }

        result
    }

    /// Calculate condensation level based on current token usage
    fn calculate_condensation_level(&self, current_tokens: usize) -> CondensationLevel {
        let ratio = current_tokens as f64 / self.config.max_tokens as f64;

        if ratio >= self.config.condensation_threshold {
            CondensationLevel::MustCondense
        } else if ratio >= self.config.warning_threshold {
            CondensationLevel::Warning
        } else {
            CondensationLevel::Ok
        }
    }

    /// Truncate items to fit within budget
    fn truncate_to_budget(
        &self,
        items: Vec<PipelineContextItem>,
    ) -> (Vec<PipelineContextItem>, bool, Vec<String>) {
        let target_tokens =
            (self.config.max_tokens as f64 * self.config.warning_threshold) as usize;

        let mut remaining: Vec<PipelineContextItem> = Vec::new();
        let mut removed: Vec<String> = Vec::new();
        let mut current_tokens: usize = 0;

        for item in items {
            if current_tokens + item.tokens <= target_tokens {
                remaining.push(item.clone());
                current_tokens += item.tokens;
            } else {
                removed.push(item.id.clone());
            }
        }

        let was_truncated = !removed.is_empty();
        (remaining, was_truncated, removed)
    }

    /// Get the current config
    pub fn config(&self) -> &ContextPipelineConfig {
        &self.config
    }

    /// Get utilization percentage
    pub fn utilization(&self, current_tokens: usize) -> f64 {
        current_tokens as f64 / self.config.max_tokens as f64
    }

    /// Check if condensation is needed
    pub fn needs_condensation(&self, current_tokens: usize) -> CondensationLevel {
        self.calculate_condensation_level(current_tokens)
    }
}

// ============================================================================
// Tier Builder - Helper to build items for each tier
// ============================================================================

/// Helper struct to build context items for each tier
pub struct TierBuilder {
    items: Vec<PipelineContextItem>,
}

impl TierBuilder {
    /// Create a new tier builder
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Add an active file item
    pub fn with_active_file(mut self, file_path: &str, content: &str) -> Self {
        let mut item = PipelineContextItem::new(
            content.to_string(),
            ContextTier::ActiveFile,
            1.0, // Highest relevance
        );
        item = item.with_source_id(file_path.to_string());
        item = item.with_priority(100);
        item = item.with_recent(true);
        self.items.push(item);
        self
    }

    /// Add an open/recent file item
    pub fn with_open_recent(
        mut self,
        file_path: &str,
        content: &str,
        relevance_score: f32,
        is_recent: bool,
    ) -> Self {
        let mut item = PipelineContextItem::new(
            content.to_string(),
            ContextTier::OpenRecent,
            relevance_score,
        );
        item = item.with_source_id(file_path.to_string());
        item = item.with_priority(if is_recent { 80 } else { 50 });
        item = item.with_recent(is_recent);
        self.items.push(item);
        self
    }

    /// Add a vector search result
    pub fn with_vector_result(mut self, id: &str, content: &str, relevance_score: f32) -> Self {
        let mut item = PipelineContextItem::new(
            content.to_string(),
            ContextTier::VectorSearch,
            relevance_score,
        );
        item = item.with_source_id(id.to_string());
        item = item.with_priority((relevance_score * 100.0) as u32);
        self.items.push(item);
        self
    }

    /// Add a graph-expanded context item
    pub fn with_graph_expanded(mut self, entity_id: &str, content: &str, distance: usize) -> Self {
        // Lower relevance for items further from the seed
        let relevance_score = (1.0 / (distance as f32 + 1.0)).max(0.1);
        let mut item = PipelineContextItem::new(
            content.to_string(),
            ContextTier::GraphExpanded,
            relevance_score,
        );
        item = item.with_source_id(entity_id.to_string());
        item = item.with_priority((relevance_score * 50.0) as u32);
        self.items.push(item);
        self
    }

    /// Add a conversation history item
    pub fn with_conversation(
        mut self,
        id: &str,
        content: &str,
        timestamp: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Self {
        let relevance_score = 0.5; // Default relevance for conversation
        let mut item = PipelineContextItem::new(
            content.to_string(),
            ContextTier::ConversationHistory,
            relevance_score,
        );
        item = item.with_source_id(id.to_string());

        // Higher priority for recent conversations
        if let Some(ts) = timestamp {
            let age_hours = (chrono::Utc::now() - ts).num_hours();
            let priority = if age_hours < 1 {
                80
            } else if age_hours < 24 {
                60
            } else {
                40
            };
            item = item.with_priority(priority);
        }

        self.items.push(item);
        self
    }

    /// Consume self and return the built items
    pub fn build(self) -> Vec<PipelineContextItem> {
        self.items
    }
}

impl Default for TierBuilder {
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

    fn create_test_item(
        content: &str,
        tier: ContextTier,
        relevance: f32,
        priority: u32,
    ) -> PipelineContextItem {
        PipelineContextItem::new(content.to_string(), tier, relevance).with_priority(priority)
    }

    #[test]
    fn test_context_pipeline_config_default() {
        let config = ContextPipelineConfig::default();
        assert_eq!(config.max_tokens, 100_000);
        assert_eq!(config.warning_threshold, 0.65);
        assert_eq!(config.condensation_threshold, 0.75);
        assert_eq!(config.tier_weights.len(), 5);
    }

    #[test]
    fn test_context_tier_ordering() {
        assert!(ContextTier::ActiveFile < ContextTier::OpenRecent);
        assert!(ContextTier::OpenRecent < ContextTier::VectorSearch);
        assert!(ContextTier::VectorSearch < ContextTier::GraphExpanded);
        assert!(ContextTier::GraphExpanded < ContextTier::ConversationHistory);
    }

    #[test]
    fn test_pipeline_context_item_priority() {
        let config = ContextPipelineConfig::default();
        let item = PipelineContextItem::new("test".to_string(), ContextTier::ActiveFile, 1.0);
        assert!(item.priority_score(&config) > 0);
    }

    #[test]
    fn test_tier_builder_active_file() {
        let items = TierBuilder::new()
            .with_active_file("src/main.rs", "fn main() {}")
            .build();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].tier, ContextTier::ActiveFile);
        assert_eq!(items[0].relevance_score, 1.0);
        assert_eq!(items[0].source_id, Some("src/main.rs".to_string()));
    }

    #[test]
    fn test_tier_builder_multiple_tiers() {
        let items = TierBuilder::new()
            .with_active_file("src/main.rs", "fn main() {}")
            .with_open_recent("src/lib.rs", "lib content", 0.9, true)
            .with_vector_result("vec-1", "vector content", 0.7)
            .with_graph_expanded("entity-1", "graph content", 1)
            .with_conversation("conv-1", "conversation content", None)
            .build();

        assert_eq!(items.len(), 5);
        assert_eq!(items[0].tier, ContextTier::ActiveFile);
        assert_eq!(items[1].tier, ContextTier::OpenRecent);
        assert_eq!(items[2].tier, ContextTier::VectorSearch);
        assert_eq!(items[3].tier, ContextTier::GraphExpanded);
        assert_eq!(items[4].tier, ContextTier::ConversationHistory);
    }

    #[test]
    fn test_context_assembler_empty() {
        let assembler = ContextAssembler::with_default_config();
        let result = assembler.assemble(Vec::new());

        assert!(result.items.is_empty());
        assert_eq!(result.total_tokens, 0);
        assert!(!result.was_truncated);
        assert!(matches!(result.condensation_level, CondensationLevel::Ok));
    }

    #[test]
    fn test_context_assembler_single_item() {
        let assembler = ContextAssembler::with_default_config();
        let items = vec![create_test_item(
            "content",
            ContextTier::ActiveFile,
            1.0,
            50,
        )];

        let result = assembler.assemble(items);

        assert_eq!(result.items.len(), 1);
        assert!(!result.was_truncated);
    }

    #[test]
    fn test_context_assembler_respects_tier_ordering() {
        let assembler = ContextAssembler::with_default_config();
        let items = vec![
            create_test_item("low_priority", ContextTier::ConversationHistory, 0.5, 20),
            create_test_item("high_priority", ContextTier::ActiveFile, 1.0, 100),
        ];

        let result = assembler.assemble(items);

        assert_eq!(result.items.len(), 2);
        // High priority should come first
        assert_eq!(result.items[0].content, "high_priority");
        assert_eq!(result.items[1].content, "low_priority");
    }

    #[test]
    fn test_context_assembler_budget_enforcement() {
        let config = ContextPipelineConfig {
            max_tokens: 1000,
            auto_compaction_enabled: true,
            ..Default::default()
        };

        let assembler = ContextAssembler::new(config);
        // Total: 400 + 300 + 200 + 150 + 100 = 1150 tokens, max is 1000 - should truncate
        let items = vec![
            PipelineContextItem::with_explicit_tokens(
                "active file content".to_string(),
                ContextTier::ActiveFile,
                1.0,
                400, // 400 tokens
            ),
            PipelineContextItem::with_explicit_tokens(
                "recent file content".to_string(),
                ContextTier::OpenRecent,
                0.8,
                300, // 300 tokens
            ),
            PipelineContextItem::with_explicit_tokens(
                "vector search result".to_string(),
                ContextTier::VectorSearch,
                0.6,
                200, // 200 tokens
            ),
            PipelineContextItem::with_explicit_tokens(
                "graph expanded".to_string(),
                ContextTier::GraphExpanded,
                0.4,
                150, // 150 tokens
            ),
            PipelineContextItem::with_explicit_tokens(
                "conversation history".to_string(),
                ContextTier::ConversationHistory,
                0.2,
                100, // 100 tokens
            ),
        ];

        let result = assembler.assemble(items);

        // Should have truncated to fit budget
        assert!(
            result.was_truncated,
            "Should truncate when total exceeds max_tokens"
        );
        assert!(result.total_tokens < 1150);
    }

    #[test]
    fn test_context_assembler_condensation_level() {
        let config = ContextPipelineConfig {
            max_tokens: 100_000,
            warning_threshold: 0.65,
            condensation_threshold: 0.75,
            ..Default::default()
        };

        let assembler = ContextAssembler::new(config);

        // At 50% - should be OK (50,000 tokens = 50% of 100k)
        let items_50 = vec![PipelineContextItem::with_explicit_tokens(
            "x".repeat(200000),
            ContextTier::ActiveFile,
            1.0,
            50000, // 50k tokens
        )];
        let result_50 = assembler.assemble(items_50);
        assert!(matches!(
            result_50.condensation_level,
            CondensationLevel::Ok
        ));

        // At 70% - should be Warning (70,000 tokens = 70% of 100k)
        let items_70 = vec![PipelineContextItem::with_explicit_tokens(
            "x".repeat(280000),
            ContextTier::ActiveFile,
            1.0,
            70000, // 70k tokens
        )];
        let result_70 = assembler.assemble(items_70);
        assert!(matches!(
            result_70.condensation_level,
            CondensationLevel::Warning
        ));

        // At 80% - should be MustCondense (80,000 tokens = 80% of 100k)
        let items_80 = vec![PipelineContextItem::with_explicit_tokens(
            "x".repeat(320000),
            ContextTier::ActiveFile,
            1.0,
            80000, // 80k tokens
        )];
        let result_80 = assembler.assemble(items_80);
        assert!(matches!(
            result_80.condensation_level,
            CondensationLevel::MustCondense
        ));
    }

    #[test]
    fn test_context_assembler_max_per_tier() {
        let config = ContextPipelineConfig {
            max_items_per_tier: 2,
            auto_compaction_enabled: false,
            ..Default::default()
        };

        let assembler = ContextAssembler::new(config);

        // Add 5 items in same tier
        let items: Vec<_> = (0..5)
            .map(|i| create_test_item(&format!("item{}", i), ContextTier::OpenRecent, 0.8, 50))
            .collect();

        let result = assembler.assemble(items);

        // Should only keep max_items_per_tier
        assert_eq!(result.items.len(), 2);
    }

    #[test]
    fn test_context_assembler_utilization() {
        let config = ContextPipelineConfig::default();
        let assembler = ContextAssembler::new(config);

        assert!((assembler.utilization(50000) - 0.5).abs() < 0.01);
        assert!((assembler.utilization(75000) - 0.75).abs() < 0.01);
    }

    #[test]
    fn test_context_assembler_needs_condensation() {
        let config = ContextPipelineConfig::default();
        let assembler = ContextAssembler::new(config);

        assert!(matches!(
            assembler.needs_condensation(50000),
            CondensationLevel::Ok
        ));
        assert!(matches!(
            assembler.needs_condensation(70000),
            CondensationLevel::Warning
        ));
        assert!(matches!(
            assembler.needs_condensation(80000),
            CondensationLevel::MustCondense
        ));
    }

    #[test]
    fn test_removed_items_tracked() {
        let config = ContextPipelineConfig {
            max_tokens: 500,
            auto_compaction_enabled: true,
            ..Default::default()
        };

        let assembler = ContextAssembler::new(config);
        // Total: 300 + 150 + 100 = 550 tokens, max is 500 - should truncate
        // With warning_threshold=0.65, target = 500 * 0.65 = 325 tokens
        // Only the first item (300 tokens) fits in target, others are removed
        let items = vec![
            PipelineContextItem::with_explicit_tokens(
                "important content here".to_string(),
                ContextTier::ActiveFile,
                1.0,
                300, // 300 tokens - highest priority, should be kept
            ),
            PipelineContextItem::with_explicit_tokens(
                "less important content".to_string(),
                ContextTier::OpenRecent,
                0.8,
                150, // 150 tokens - second priority, should be removed
            ),
            PipelineContextItem::with_explicit_tokens(
                "unimportant content".to_string(),
                ContextTier::ConversationHistory,
                0.2,
                100, // 100 tokens - lowest priority, should be removed
            ),
        ];

        let result = assembler.assemble(items);

        assert!(result.was_truncated);
        assert!(!result.removed_items.is_empty());
        // Only 1 item should remain (300 tokens fits in target of 325)
        assert_eq!(
            result.items.len(),
            1,
            "Should have kept only the highest priority item"
        );
        // Check that the removed items are tracked
        assert_eq!(result.removed_items.len(), 2, "Should have removed 2 items");
    }

    #[test]
    fn test_tier_builder_conversation_with_timestamp() {
        use chrono::Duration;

        let recent = chrono::Utc::now() - Duration::minutes(30);
        let old = chrono::Utc::now() - Duration::hours(48);

        let recent_items = TierBuilder::new()
            .with_conversation("recent", "recent content", Some(recent))
            .build();

        let old_items = TierBuilder::new()
            .with_conversation("old", "old content", Some(old))
            .build();

        // Recent should have higher priority
        assert!(recent_items[0].priority > old_items[0].priority);
    }

    #[test]
    fn test_graph_expanded_distance_scoring() {
        let items = [
            TierBuilder::new()
                .with_graph_expanded("entity", "distance 0", 0)
                .build(),
            TierBuilder::new()
                .with_graph_expanded("entity", "distance 1", 1)
                .build(),
            TierBuilder::new()
                .with_graph_expanded("entity", "distance 2", 2)
                .build(),
        ];

        // Distance 0 should have highest relevance
        assert!(items[0][0].relevance_score > items[1][0].relevance_score);
        assert!(items[1][0].relevance_score > items[2][0].relevance_score);
    }

    #[test]
    fn test_auto_compaction_disabled() {
        let config = ContextPipelineConfig {
            auto_compaction_enabled: false,
            max_tokens: 100,
            ..Default::default()
        };

        let assembler = ContextAssembler::new(config);
        let items = vec![
            create_test_item("item1", ContextTier::ActiveFile, 1.0, 100),
            create_test_item("item2", ContextTier::OpenRecent, 0.8, 80),
        ];

        let result = assembler.assemble(items);

        // Should not truncate even if over budget
        assert!(!result.was_truncated);
    }
}
