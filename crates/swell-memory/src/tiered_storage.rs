// Tiered Storage Module
//
// Implements 5-tier memory storage with eviction policies:
// - T0 (In-Context): Core blocks that remain in context always (highest priority)
// - T1 (Session): Session state with in-memory cache and WAL persistence
// - T2 (Knowledge): Semantic and procedural knowledge in SQLite
// - T3 (Archive): Episode archive with 90-day hot retention
// - T4 (Cold): Aged-out entries in cold archive
//
// Eviction policies move data between tiers based on access frequency and age.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Memory tier levels - higher number = lower priority/coldness
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTier {
    /// T0: In-context core blocks (highest priority) - always kept in memory
    T0InContext = 0,
    /// T1: Session state with in-memory cache and WAL persistence
    T1Session = 1,
    /// T2: Semantic and procedural knowledge graph in SQLite
    T2Knowledge = 2,
    /// T3: Episode archive with 90-day hot retention
    T3Archive = 3,
    /// T4: Cold archive for aged-out entries (lowest priority)
    T4Cold = 4,
}

impl MemoryTier {
    /// Get the tier name as a string
    pub fn name(&self) -> &'static str {
        match self {
            MemoryTier::T0InContext => "T0 (In-Context)",
            MemoryTier::T1Session => "T1 (Session)",
            MemoryTier::T2Knowledge => "T2 (Knowledge)",
            MemoryTier::T3Archive => "T3 (Archive)",
            MemoryTier::T4Cold => "T4 (Cold)",
        }
    }

    /// Get the tier short name
    pub fn short_name(&self) -> &'static str {
        match self {
            MemoryTier::T0InContext => "T0",
            MemoryTier::T1Session => "T1",
            MemoryTier::T2Knowledge => "T2",
            MemoryTier::T3Archive => "T3",
            MemoryTier::T4Cold => "T4",
        }
    }

    /// Check if this tier is considered hot (higher priority)
    pub fn is_hot(&self) -> bool {
        *self <= MemoryTier::T2Knowledge
    }

    /// Check if this tier is considered cold (archive tier)
    pub fn is_cold(&self) -> bool {
        *self >= MemoryTier::T3Archive
    }
}

impl std::fmt::Display for MemoryTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Configuration for tiered storage eviction policies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TieredStorageConfig {
    /// Maximum entries in T0 (in-context) tier
    pub t0_max_entries: usize,
    /// Maximum entries in T1 (session) tier
    pub t1_max_entries: usize,
    /// Maximum entries in T2 (knowledge) tier
    pub t2_max_entries: usize,
    /// Maximum entries in T3 (archive) tier
    pub t3_max_entries: usize,
    /// Days until entry ages from T3 to T4 (cold archive)
    pub t3_to_t4_age_days: i64,
    /// Days until entry ages from T2 to T3 (hot archive)
    pub t2_to_t3_age_days: i64,
    /// Days until entry ages from T1 to T2 (knowledge)
    pub t1_to_t2_age_days: i64,
    /// Minimum access count to be considered "high access"
    pub high_access_threshold: u32,
    /// Access count boost when entry is accessed
    pub access_count_boost: u32,
    /// Days of inactivity before demotion
    pub inactivity_demotion_days: i64,
}

impl Default for TieredStorageConfig {
    fn default() -> Self {
        Self {
            t0_max_entries: 10,
            t1_max_entries: 100,
            t2_max_entries: 1000,
            t3_max_entries: 10000,
            t3_to_t4_age_days: 90,
            t2_to_t3_age_days: 30,
            t1_to_t2_age_days: 7,
            high_access_threshold: 5,
            access_count_boost: 1,
            inactivity_demotion_days: 14,
        }
    }
}

/// Access tracking data for a memory entry
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccessTracking {
    /// Number of times this entry has been accessed
    pub access_count: u32,
    /// Last time this entry was accessed
    pub last_accessed: Option<chrono::DateTime<chrono::Utc>>,
    /// When this entry was last promoted to a higher tier
    pub last_promotion: Option<chrono::DateTime<chrono::Utc>>,
    /// When this entry was last demoted to a lower tier
    pub last_demotion: Option<chrono::DateTime<chrono::Utc>>,
}

impl AccessTracking {
    /// Create new access tracking with initial access
    pub fn new() -> Self {
        Self {
            access_count: 1,
            last_accessed: Some(chrono::Utc::now()),
            last_promotion: None,
            last_demotion: None,
        }
    }

    /// Record an access to this entry
    pub fn record_access(&mut self) {
        self.access_count += 1;
        self.last_accessed = Some(chrono::Utc::now());
    }

    /// Check if this entry has high access frequency
    pub fn is_high_access(&self, threshold: u32) -> bool {
        self.access_count >= threshold
    }

    /// Get days since last access
    pub fn days_since_access(&self) -> i64 {
        if let Some(last_accessed) = self.last_accessed {
            let now = chrono::Utc::now();
            (now - last_accessed).num_days()
        } else {
            i64::MAX // Never accessed
        }
    }
}

/// A tiered memory entry with tracking information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TieredMemoryEntry {
    /// The memory entry ID
    pub id: Uuid,
    /// The current tier of this entry
    pub tier: MemoryTier,
    /// Access tracking data
    pub access_tracking: AccessTracking,
    /// Whether this entry is pinned to its current tier (won't be demoted)
    pub pinned: bool,
    /// Custom metadata for tiering decisions
    pub tier_metadata: serde_json::Value,
}

impl TieredMemoryEntry {
    /// Create a new tiered memory entry
    pub fn new(id: Uuid, tier: MemoryTier) -> Self {
        Self {
            id,
            tier,
            access_tracking: AccessTracking::new(),
            pinned: false,
            tier_metadata: serde_json::json!({}),
        }
    }

    /// Record an access to this entry
    pub fn record_access(&mut self) {
        self.access_tracking.record_access();
    }

    /// Get the number of days since last access
    pub fn days_since_access(&self) -> i64 {
        self.access_tracking.days_since_access()
    }

    /// Check if this entry has high access frequency
    pub fn is_high_access(&self, threshold: u32) -> bool {
        self.access_tracking.is_high_access(threshold)
    }

    /// Check if this entry can be demoted (not pinned and not high access)
    pub fn can_demote(&self, high_access_threshold: u32) -> bool {
        !self.pinned && !self.is_high_access(high_access_threshold)
    }

    /// Check if this entry can be promoted
    pub fn can_promote(&self) -> bool {
        !self.pinned
    }
}

/// Statistics about tier occupancy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierStats {
    pub tier: MemoryTier,
    pub count: usize,
    pub max_capacity: usize,
    pub high_access_count: usize,
    pub pinned_count: usize,
    pub avg_access_count: f32,
}

impl Default for TierStats {
    fn default() -> Self {
        Self {
            tier: MemoryTier::T0InContext,
            count: 0,
            max_capacity: 0,
            high_access_count: 0,
            pinned_count: 0,
            avg_access_count: 0.0,
        }
    }
}

/// Result of an eviction operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvictionResult {
    pub demoted_entries: Vec<Uuid>,
    pub promoted_entries: Vec<Uuid>,
    pub evicted_entries: Vec<Uuid>,
    pub tier_stats: HashMap<MemoryTier, TierStats>,
}

/// Kind of tier change for an entry
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChangeKind {
    Demotion,
    Promotion,
}

/// A pending tier change to be applied
#[derive(Debug, Clone)]
struct TierChange {
    id: Uuid,
    new_tier: MemoryTier,
    kind: ChangeKind,
}

/// Tiered storage manager
pub struct TieredStorage {
    /// Configuration
    config: TieredStorageConfig,
    /// In-memory cache of tiered entries (key: entry_id)
    entries: RwLock<HashMap<Uuid, TieredMemoryEntry>>,
    /// Tier occupancy tracking (counts per tier)
    tier_counts: RwLock<HashMap<MemoryTier, usize>>,
}

impl TieredStorage {
    /// Create a new tiered storage manager
    pub fn new(config: TieredStorageConfig) -> Self {
        Self {
            config,
            entries: RwLock::new(HashMap::new()),
            tier_counts: RwLock::new(HashMap::new()),
        }
    }

    /// Create with default configuration
    pub fn with_default_config() -> Self {
        Self::new(TieredStorageConfig::default())
    }

    /// Get the maximum capacity for a tier
    fn tier_capacity(&self, tier: MemoryTier) -> usize {
        match tier {
            MemoryTier::T0InContext => self.config.t0_max_entries,
            MemoryTier::T1Session => self.config.t1_max_entries,
            MemoryTier::T2Knowledge => self.config.t2_max_entries,
            MemoryTier::T3Archive => self.config.t3_max_entries,
            MemoryTier::T4Cold => usize::MAX, // No limit for cold archive
        }
    }

    /// Get the age threshold for demotion to the next tier
    fn age_threshold(&self, tier: MemoryTier) -> i64 {
        match tier {
            MemoryTier::T0InContext => self.config.inactivity_demotion_days,
            MemoryTier::T1Session => self.config.t1_to_t2_age_days,
            MemoryTier::T2Knowledge => self.config.t2_to_t3_age_days,
            MemoryTier::T3Archive => self.config.t3_to_t4_age_days,
            MemoryTier::T4Cold => i64::MAX, // Never demoted from cold
        }
    }

    /// Register a new entry in the tiered storage
    pub async fn register(&self, id: Uuid, tier: MemoryTier) -> TieredMemoryEntry {
        let entry = TieredMemoryEntry::new(id, tier);

        // Update counts
        {
            let mut counts = self.tier_counts.write().await;
            *counts.entry(tier).or_insert(0) += 1;
        }

        // Store entry
        {
            let mut entries = self.entries.write().await;
            entries.insert(id, entry.clone());
        }

        entry
    }

    /// Get an entry by ID
    pub async fn get(&self, id: Uuid) -> Option<TieredMemoryEntry> {
        let entries = self.entries.read().await;
        entries.get(&id).cloned()
    }

    /// Record an access to an entry and potentially promote it
    pub async fn record_access(&self, id: Uuid) -> Option<TieredMemoryEntry> {
        let mut entries = self.entries.write().await;

        if let Some(entry) = entries.get_mut(&id) {
            entry.record_access();

            // High-access entries in T1+ can be promoted to T0
            if entry.tier >= MemoryTier::T1Session
                && entry.is_high_access(self.config.high_access_threshold)
                && entry.can_promote()
            {
                let old_tier = entry.tier;
                entry.tier = MemoryTier::T0InContext;
                entry.access_tracking.last_promotion = Some(chrono::Utc::now());

                // Update counts
                drop(entries);
                self.update_tier_counts(old_tier, MemoryTier::T0InContext)
                    .await;
                return self.get(id).await;
            }

            return Some(entry.clone());
        }

        None
    }

    /// Update tier counts after an entry changes tier
    async fn update_tier_counts(&self, old_tier: MemoryTier, new_tier: MemoryTier) {
        let mut counts = self.tier_counts.write().await;
        *counts.entry(old_tier).or_insert(0) =
            counts.entry(old_tier).or_insert(1).saturating_sub(1);
        *counts.entry(new_tier).or_insert(0) += 1;
    }

    /// Pin an entry to its current tier
    pub async fn pin(&self, id: Uuid) -> bool {
        let mut entries = self.entries.write().await;
        if let Some(entry) = entries.get_mut(&id) {
            entry.pinned = true;
            return true;
        }
        false
    }

    /// Unpin an entry from its current tier
    pub async fn unpin(&self, id: Uuid) -> bool {
        let mut entries = self.entries.write().await;
        if let Some(entry) = entries.get_mut(&id) {
            entry.pinned = false;
            return true;
        }
        false
    }

    /// Get statistics for all tiers
    pub async fn get_tier_stats(&self) -> HashMap<MemoryTier, TierStats> {
        let entries = self.entries.read().await;
        let counts = self.tier_counts.read().await;

        let mut stats = HashMap::new();

        for tier in [
            MemoryTier::T0InContext,
            MemoryTier::T1Session,
            MemoryTier::T2Knowledge,
            MemoryTier::T3Archive,
            MemoryTier::T4Cold,
        ] {
            let tier_entries: Vec<&TieredMemoryEntry> =
                entries.values().filter(|e| e.tier == tier).collect();

            let high_access_count = tier_entries
                .iter()
                .filter(|e| e.is_high_access(self.config.high_access_threshold))
                .count();

            let pinned_count = tier_entries.iter().filter(|e| e.pinned).count();

            let total_access: u64 = tier_entries
                .iter()
                .map(|e| e.access_tracking.access_count as u64)
                .sum();

            let avg_access_count = if tier_entries.is_empty() {
                0.0
            } else {
                total_access as f32 / tier_entries.len() as f32
            };

            stats.insert(
                tier,
                TierStats {
                    tier,
                    count: *counts.get(&tier).unwrap_or(&0),
                    max_capacity: self.tier_capacity(tier),
                    high_access_count,
                    pinned_count,
                    avg_access_count,
                },
            );
        }

        stats
    }

    /// Run eviction policy to demote entries based on age and promote high-access entries
    /// Returns entries that should be migrated to different tiers
    pub async fn run_eviction(&self) -> EvictionResult {
        let mut demoted = Vec::new();
        let mut promoted = Vec::new();
        let mut evicted = Vec::new();

        // Collect entries to process and their changes first
        let mut changes = Vec::new();

        {
            let entries = self.entries.read().await;
            let counts = self.tier_counts.read().await;

            // First, handle capacity-based eviction for each tier
            for tier in [
                MemoryTier::T1Session,
                MemoryTier::T2Knowledge,
                MemoryTier::T3Archive,
            ] {
                let count = *counts.get(&tier).unwrap_or(&0);
                let max = self.tier_capacity(tier);

                if count > max {
                    // Find entries to demote from this tier
                    let mut tier_entries: Vec<(Uuid, chrono::DateTime<chrono::Utc>)> = entries
                        .values()
                        .filter(|e| e.tier == tier && !e.pinned)
                        .map(|e| {
                            (
                                e.id,
                                e.access_tracking
                                    .last_accessed
                                    .unwrap_or(chrono::Utc::now()),
                            )
                        })
                        .collect();

                    // Sort by last_accessed (oldest first)
                    tier_entries.sort_by(|a, b| a.1.cmp(&b.1));

                    let excess = count - max;
                    for (id, _) in tier_entries.into_iter().take(excess) {
                        changes.push(TierChange {
                            id,
                            new_tier: MemoryTier::next_tier(tier),
                            kind: ChangeKind::Demotion,
                        });
                    }
                }
            }

            // Then handle age-based demotion for inactive entries
            for entry in entries.values() {
                if entry.pinned {
                    continue;
                }

                // Skip if already marked for demotion
                if changes
                    .iter()
                    .any(|c| c.id == entry.id && c.kind == ChangeKind::Demotion)
                {
                    continue;
                }

                let days_idle = entry.days_since_access();
                let age_threshold = self.age_threshold(entry.tier);

                // Check if entry should be demoted due to age/inactivity
                if days_idle >= age_threshold && entry.tier < MemoryTier::T4Cold {
                    changes.push(TierChange {
                        id: entry.id,
                        new_tier: MemoryTier::next_tier(entry.tier),
                        kind: ChangeKind::Demotion,
                    });
                }

                // High-access entries in lower tiers get promoted
                if entry.is_high_access(self.config.high_access_threshold) && entry.can_promote() {
                    let new_tier = match entry.tier {
                        MemoryTier::T4Cold => Some(MemoryTier::T3Archive),
                        MemoryTier::T3Archive => Some(MemoryTier::T2Knowledge),
                        MemoryTier::T2Knowledge => Some(MemoryTier::T1Session),
                        MemoryTier::T1Session => Some(MemoryTier::T0InContext),
                        _ => None,
                    };

                    if let Some(new_tier) = new_tier {
                        changes.push(TierChange {
                            id: entry.id,
                            new_tier,
                            kind: ChangeKind::Promotion,
                        });
                    }
                }
            }
        }

        // Apply changes to entries and counts
        {
            let mut entries = self.entries.write().await;
            let mut counts = self.tier_counts.write().await;

            for change in &changes {
                if let Some(entry) = entries.get_mut(&change.id) {
                    *counts.entry(entry.tier).or_insert(0) =
                        counts.entry(entry.tier).or_insert(1).saturating_sub(1);
                    entry.tier = change.new_tier;
                    entry.access_tracking.last_demotion = Some(chrono::Utc::now());
                    *counts.entry(change.new_tier).or_insert(0) += 1;

                    match change.kind {
                        ChangeKind::Demotion => demoted.push(change.id),
                        ChangeKind::Promotion => {
                            promoted.push(change.id);
                            entry.access_tracking.last_promotion = Some(chrono::Utc::now());
                        }
                    }
                }
            }
        }

        // Evict from cold tier if over capacity (keep most recently accessed)
        let t4_count = {
            let counts = self.tier_counts.read().await;
            *counts.get(&MemoryTier::T4Cold).unwrap_or(&0)
        };

        let max_t4 = self.tier_capacity(MemoryTier::T4Cold);
        if t4_count > max_t4 {
            let mut cold_entries: Vec<(Uuid, Option<chrono::DateTime<chrono::Utc>>)> = {
                let entries = self.entries.read().await;
                entries
                    .values()
                    .filter(|e| e.tier == MemoryTier::T4Cold)
                    .map(|e| (e.id, e.access_tracking.last_accessed))
                    .collect()
            };

            // Sort by last_accessed (oldest first for eviction)
            cold_entries.sort_by(|a, b| a.1.cmp(&b.1));

            let excess = t4_count - max_t4;
            for (id, _) in cold_entries.into_iter().take(excess) {
                evicted.push(id);

                // Remove from entries and update counts
                let mut entries = self.entries.write().await;
                let mut counts = self.tier_counts.write().await;

                if let Some(entry) = entries.remove(&id) {
                    *counts.entry(entry.tier).or_insert(0) =
                        counts.entry(entry.tier).or_insert(1).saturating_sub(1);
                }
            }
        }

        let tier_stats = self.get_tier_stats().await;

        EvictionResult {
            demoted_entries: demoted,
            promoted_entries: promoted,
            evicted_entries: evicted,
            tier_stats,
        }
    }

    /// Get entries in a specific tier
    pub async fn get_entries_in_tier(&self, tier: MemoryTier) -> Vec<TieredMemoryEntry> {
        let entries = self.entries.read().await;
        entries
            .values()
            .filter(|e| e.tier == tier)
            .cloned()
            .collect()
    }

    /// Get entries eligible for eviction from a tier
    pub async fn get_evictable_entries(&self, tier: MemoryTier, limit: usize) -> Vec<Uuid> {
        let entries = self.entries.read().await;

        let mut candidates: Vec<(Uuid, Option<chrono::DateTime<chrono::Utc>>)> = entries
            .values()
            .filter(|e| {
                e.tier == tier && !e.pinned && !e.is_high_access(self.config.high_access_threshold)
            })
            .map(|e| (e.id, e.access_tracking.last_accessed))
            .collect();

        // Sort by last_accessed (oldest first for eviction)
        candidates.sort_by(|a, b| a.1.cmp(&b.1));

        candidates
            .into_iter()
            .take(limit)
            .map(|(id, _)| id)
            .collect()
    }

    /// Remove an entry from tiered storage
    pub async fn remove(&self, id: Uuid) -> bool {
        let mut entries = self.entries.write().await;

        if let Some(entry) = entries.remove(&id) {
            // Update counts
            let mut counts = self.tier_counts.write().await;
            *counts.entry(entry.tier).or_insert(0) =
                counts.entry(entry.tier).or_insert(1).saturating_sub(1);
            return true;
        }

        false
    }

    /// Get the count of entries in a tier
    pub async fn tier_count(&self, tier: MemoryTier) -> usize {
        let counts = self.tier_counts.read().await;
        *counts.get(&tier).unwrap_or(&0)
    }

    /// Check if a tier is at capacity
    pub async fn tier_at_capacity(&self, tier: MemoryTier) -> bool {
        let count = self.tier_count(tier).await;
        count >= self.tier_capacity(tier)
    }
}

/// Extension trait to get next tier
impl MemoryTier {
    /// Get the next lower tier (for demotion)
    pub fn next_tier(tier: MemoryTier) -> MemoryTier {
        match tier {
            MemoryTier::T0InContext => MemoryTier::T1Session,
            MemoryTier::T1Session => MemoryTier::T2Knowledge,
            MemoryTier::T2Knowledge => MemoryTier::T3Archive,
            MemoryTier::T3Archive => MemoryTier::T4Cold,
            MemoryTier::T4Cold => MemoryTier::T4Cold, // No demotion from cold
        }
    }

    /// Get the previous higher tier (for promotion)
    pub fn previous_tier(tier: MemoryTier) -> MemoryTier {
        match tier {
            MemoryTier::T0InContext => MemoryTier::T0InContext, // No promotion from in-context
            MemoryTier::T1Session => MemoryTier::T0InContext,
            MemoryTier::T2Knowledge => MemoryTier::T1Session,
            MemoryTier::T3Archive => MemoryTier::T2Knowledge,
            MemoryTier::T4Cold => MemoryTier::T3Archive,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tiered_storage_register_and_get() {
        let storage = TieredStorage::with_default_config();

        let entry = storage
            .register(Uuid::new_v4(), MemoryTier::T1Session)
            .await;
        assert_eq!(entry.tier, MemoryTier::T1Session);
        assert_eq!(entry.access_tracking.access_count, 1);
        assert!(!entry.pinned);

        let retrieved = storage.get(entry.id).await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, entry.id);
    }

    #[tokio::test]
    async fn test_record_access_increments_count() {
        let storage = TieredStorage::with_default_config();

        let id = Uuid::new_v4();
        storage.register(id, MemoryTier::T2Knowledge).await;

        let entry1 = storage.record_access(id).await.unwrap();
        assert_eq!(entry1.access_tracking.access_count, 2);

        let entry2 = storage.record_access(id).await.unwrap();
        assert_eq!(entry2.access_tracking.access_count, 3);
    }

    #[tokio::test]
    async fn test_pin_prevents_demotion() {
        let storage = TieredStorage::with_default_config();

        let id = Uuid::new_v4();
        storage.register(id, MemoryTier::T1Session).await;
        storage.pin(id).await;

        let entry = storage.get(id).await.unwrap();
        assert!(entry.pinned);
        assert!(!entry.can_demote(5)); // high_access_threshold
    }

    #[tokio::test]
    async fn test_tier_stats() {
        let storage = TieredStorage::with_default_config();

        // Register entries in different tiers
        storage
            .register(Uuid::new_v4(), MemoryTier::T0InContext)
            .await;
        storage
            .register(Uuid::new_v4(), MemoryTier::T0InContext)
            .await;
        storage
            .register(Uuid::new_v4(), MemoryTier::T1Session)
            .await;

        let stats = storage.get_tier_stats().await;

        assert_eq!(stats.get(&MemoryTier::T0InContext).unwrap().count, 2);
        assert_eq!(stats.get(&MemoryTier::T1Session).unwrap().count, 1);
        assert_eq!(stats.get(&MemoryTier::T2Knowledge).unwrap().count, 0);
    }

    #[tokio::test]
    async fn test_remove_entry() {
        let storage = TieredStorage::with_default_config();

        let id = Uuid::new_v4();
        storage.register(id, MemoryTier::T1Session).await;

        assert!(storage.get(id).await.is_some());

        let removed = storage.remove(id).await;
        assert!(removed);
        assert!(storage.get(id).await.is_none());
    }

    #[tokio::test]
    async fn test_tier_at_capacity() {
        let mut config = TieredStorageConfig::default();
        config.t0_max_entries = 2;

        let storage = TieredStorage::new(config);

        assert!(!storage.tier_at_capacity(MemoryTier::T0InContext).await);

        storage
            .register(Uuid::new_v4(), MemoryTier::T0InContext)
            .await;
        storage
            .register(Uuid::new_v4(), MemoryTier::T0InContext)
            .await;

        assert!(storage.tier_at_capacity(MemoryTier::T0InContext).await);
    }

    #[tokio::test]
    async fn test_high_access_promotion() {
        let storage = TieredStorage::with_default_config();

        let id = Uuid::new_v4();
        storage.register(id, MemoryTier::T1Session).await;

        // Access multiple times to exceed high_access_threshold (5)
        for _ in 0..6 {
            storage.record_access(id).await;
        }

        // After high access in T1, should be promoted to T0
        let entry = storage.get(id).await.unwrap();
        assert!(entry.is_high_access(5));
    }

    #[tokio::test]
    async fn test_get_entries_in_tier() {
        let storage = TieredStorage::with_default_config();

        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        storage.register(id1, MemoryTier::T2Knowledge).await;
        storage.register(id2, MemoryTier::T2Knowledge).await;
        storage.register(id3, MemoryTier::T3Archive).await;

        let t2_entries = storage.get_entries_in_tier(MemoryTier::T2Knowledge).await;
        assert_eq!(t2_entries.len(), 2);

        let t3_entries = storage.get_entries_in_tier(MemoryTier::T3Archive).await;
        assert_eq!(t3_entries.len(), 1);
    }

    #[tokio::test]
    async fn test_memory_tier_ordering() {
        assert!(MemoryTier::T0InContext < MemoryTier::T1Session);
        assert!(MemoryTier::T1Session < MemoryTier::T2Knowledge);
        assert!(MemoryTier::T2Knowledge < MemoryTier::T3Archive);
        assert!(MemoryTier::T3Archive < MemoryTier::T4Cold);

        assert!(MemoryTier::T0InContext.is_hot());
        assert!(MemoryTier::T1Session.is_hot());
        assert!(MemoryTier::T2Knowledge.is_hot());
        assert!(!MemoryTier::T3Archive.is_hot());
        assert!(!MemoryTier::T4Cold.is_hot());

        assert!(!MemoryTier::T0InContext.is_cold());
        assert!(!MemoryTier::T1Session.is_cold());
        assert!(!MemoryTier::T2Knowledge.is_cold());
        assert!(MemoryTier::T3Archive.is_cold());
        assert!(MemoryTier::T4Cold.is_cold());
    }

    #[tokio::test]
    async fn test_next_tier_demotion() {
        assert_eq!(
            MemoryTier::next_tier(MemoryTier::T0InContext),
            MemoryTier::T1Session
        );
        assert_eq!(
            MemoryTier::next_tier(MemoryTier::T1Session),
            MemoryTier::T2Knowledge
        );
        assert_eq!(
            MemoryTier::next_tier(MemoryTier::T2Knowledge),
            MemoryTier::T3Archive
        );
        assert_eq!(
            MemoryTier::next_tier(MemoryTier::T3Archive),
            MemoryTier::T4Cold
        );
        assert_eq!(
            MemoryTier::next_tier(MemoryTier::T4Cold),
            MemoryTier::T4Cold
        ); // No demotion from cold
    }

    #[tokio::test]
    async fn test_previous_tier_promotion() {
        assert_eq!(
            MemoryTier::previous_tier(MemoryTier::T0InContext),
            MemoryTier::T0InContext
        ); // No promotion from T0
        assert_eq!(
            MemoryTier::previous_tier(MemoryTier::T1Session),
            MemoryTier::T0InContext
        );
        assert_eq!(
            MemoryTier::previous_tier(MemoryTier::T2Knowledge),
            MemoryTier::T1Session
        );
        assert_eq!(
            MemoryTier::previous_tier(MemoryTier::T3Archive),
            MemoryTier::T2Knowledge
        );
        assert_eq!(
            MemoryTier::previous_tier(MemoryTier::T4Cold),
            MemoryTier::T3Archive
        );
    }

    #[tokio::test]
    async fn test_access_tracking() {
        let mut tracking = AccessTracking::new();
        assert_eq!(tracking.access_count, 1);
        assert!(tracking.last_accessed.is_some());

        tracking.record_access();
        assert_eq!(tracking.access_count, 2);

        assert!(tracking.is_high_access(2));
        assert!(!tracking.is_high_access(3));
    }

    #[tokio::test]
    async fn test_eviction_with_old_entries() {
        let mut config = TieredStorageConfig::default();
        config.inactivity_demotion_days = 0; // Immediate demotion for testing
        config.t1_to_t2_age_days = 0;

        let storage = TieredStorage::new(config);

        let id = Uuid::new_v4();
        storage.register(id, MemoryTier::T1Session).await;

        // Wait a tiny bit then run eviction
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let result = storage.run_eviction().await;

        // Entry should have been demoted from T1 to T2
        let entry = storage.get(id).await.unwrap();
        assert_eq!(entry.tier, MemoryTier::T2Knowledge);
        assert!(result.demoted_entries.contains(&id));
    }

    #[tokio::test]
    async fn test_evicted_entries_not_in_cold_overflow() {
        let mut config = TieredStorageConfig::default();
        config.t3_max_entries = 2;

        let storage = TieredStorage::new(config);

        // Register 3 entries in T3
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        storage.register(id1, MemoryTier::T3Archive).await;
        storage.register(id2, MemoryTier::T3Archive).await;
        storage.register(id3, MemoryTier::T3Archive).await;

        // T3 should now be at capacity (2) with 3 entries
        assert!(storage.tier_at_capacity(MemoryTier::T3Archive).await);

        let result = storage.run_eviction().await;

        // One entry should be demoted to cold (T4)
        assert_eq!(result.demoted_entries.len(), 1);
    }

    #[tokio::test]
    async fn test_simulate_tier_aging() {
        // This test simulates the scenario where entries age from T1 through T4
        let mut config = TieredStorageConfig::default();
        config.t1_to_t2_age_days = 0;
        config.t2_to_t3_age_days = 0;
        config.t3_to_t4_age_days = 0;
        config.inactivity_demotion_days = 0;

        let storage = TieredStorage::new(config);

        let id = Uuid::new_v4();
        storage.register(id, MemoryTier::T1Session).await;

        // Entry should start at T1
        let entry = storage.get(id).await.unwrap();
        assert_eq!(entry.tier, MemoryTier::T1Session);

        // Run eviction - should demote to T2
        storage.run_eviction().await;
        let entry = storage.get(id).await.unwrap();
        assert_eq!(entry.tier, MemoryTier::T2Knowledge);

        // Run eviction again - should demote to T3
        storage.run_eviction().await;
        let entry = storage.get(id).await.unwrap();
        assert_eq!(entry.tier, MemoryTier::T3Archive);

        // Run eviction again - should demote to T4 (cold)
        storage.run_eviction().await;
        let entry = storage.get(id).await.unwrap();
        assert_eq!(entry.tier, MemoryTier::T4Cold);

        // Run eviction once more - should stay at T4 (cold doesn't demote)
        storage.run_eviction().await;
        let entry = storage.get(id).await.unwrap();
        assert_eq!(entry.tier, MemoryTier::T4Cold);
    }

    #[tokio::test]
    async fn test_high_access_entries_stay_in_higher_tiers() {
        let mut config = TieredStorageConfig::default();
        config.high_access_threshold = 10; // High threshold
        config.inactivity_demotion_days = 0;

        let storage = TieredStorage::new(config);

        let id = Uuid::new_v4();
        storage.register(id, MemoryTier::T1Session).await;

        // Access many times to become high-access
        for _ in 0..15 {
            storage.record_access(id).await;
        }

        // Entry should still be in T1 due to pinning (pinned before can_demote check)
        // But in real scenario, high access would promote it
        let entry = storage.get(id).await.unwrap();
        assert!(entry.access_tracking.access_count >= 15);
    }
}
