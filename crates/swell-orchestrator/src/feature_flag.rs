//! Feature flag infrastructure for instant rollback and gradual rollouts.
//!
//! This module provides:
//! - [`FeatureFlag`] - represents a single feature flag with rollout configuration
//! - [`FeatureFlagManager`] - manages all feature flags with rollback support
//! - [`FlagSnapshot`] - snapshot of flag state for instant rollback capability
//!
//! # Example
//!
//! ```rust
//! use swell_orchestrator::{FeatureFlagManager, FeatureFlag};
//!
//! let manager = FeatureFlagManager::new();
//!
//! // Create a flag with 0% rollout (disabled by default)
//! manager.create_flag("my_feature", "My feature description", 0).unwrap();
//!
//! // Enable the flag immediately
//! manager.enable("my_feature").unwrap();
//!
//! // Check if enabled (will respect rollout percentage)
//! assert!(manager.is_enabled("my_feature", None));
//!
//! // Take a snapshot before risky change
//! manager.snapshot("my_feature").unwrap();
//!
//! // Make changes...
//! manager.set_rollout("my_feature", 50).unwrap();
//!
//! // Rollback if issues arise
//! manager.rollback("my_feature").unwrap();
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;
use tracing::{info, warn};
use uuid::Uuid;

/// Errors that can occur during feature flag operations
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum FeatureFlagError {
    #[error("Flag '{0}' not found")]
    FlagNotFound(String),

    #[error("Flag '{0}' already exists")]
    FlagAlreadyExists(String),

    #[error("Rollout percentage must be between 0 and 100, got {0}")]
    InvalidRolloutPercentage(u8),

    #[error("No snapshot available for flag '{0}'")]
    NoSnapshot(String),

    #[error("Invalid flag name: {0}")]
    InvalidFlagName(String),
}

/// Represents a feature flag with rollout configuration
///
/// A feature flag can be:
/// - Fully disabled (rollout = 0)
/// - Fully enabled (rollout = 100)
/// - Partially enabled (rollout = 1-99, gradual rollout)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureFlag {
    /// Unique name identifying this flag
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Whether the flag is enabled at all (global on/off switch)
    enabled: bool,
    /// Rollout percentage: 0-100
    /// 0 = disabled for everyone
    /// 100 = enabled for everyone
    /// 1-99 = enabled for that percentage of users (gradual rollout)
    rollout_percentage: u8,
    /// When the flag was created
    pub created_at: DateTime<Utc>,
    /// When the flag was last modified
    pub updated_at: DateTime<Utc>,
    /// Additional metadata for the flag
    pub metadata: serde_json::Value,
    /// Rollout strategy (for future extensibility)
    pub strategy: RolloutStrategy,
}

/// Rollout strategy for gradual feature releases
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RolloutStrategy {
    /// Random percentage rollout (default)
    #[default]
    Random,
    /// Rollout based on user ID hash (consistent experience for same user)
    UserHash,
    /// Rollout based on task ID hash
    TaskHash,
    /// Phased rollout with specific phases
    Phased { phases: Vec<RolloutPhase> },
}

/// A single phase in a phased rollout
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RolloutPhase {
    /// Percentage for this phase
    pub percentage: u8,
    /// Minimum iteration count to be eligible
    pub min_iterations: u32,
}

/// Snapshot of a feature flag state for rollback capability
///
/// Stores the complete state of a flag at a point in time,
/// allowing instant rollback to a known good state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlagSnapshot {
    /// UUID identifying this snapshot
    pub id: Uuid,
    /// Name of the flag this snapshot belongs to
    pub flag_name: String,
    /// Complete state of the flag at snapshot time
    pub flag_state: FeatureFlag,
    /// When the snapshot was created
    pub created_at: DateTime<Utc>,
    /// Optional description of why the snapshot was taken
    pub description: Option<String>,
}

impl FlagSnapshot {
    /// Create a new snapshot for a flag
    pub fn new(flag: &FeatureFlag, description: Option<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            flag_name: flag.name.clone(),
            flag_state: flag.clone(),
            created_at: Utc::now(),
            description,
        }
    }
}

impl FeatureFlag {
    /// Create a new feature flag
    ///
    /// # Arguments
    /// * `name` - Unique identifier for the flag
    /// * `description` - Human-readable description
    /// * `rollout_percentage` - Initial rollout (0-100)
    ///
    /// # Errors
    /// Returns `FeatureFlagError::InvalidRolloutPercentage` if rollout is > 100
    pub fn new(
        name: String,
        description: String,
        rollout_percentage: u8,
    ) -> Result<Self, FeatureFlagError> {
        if rollout_percentage > 100 {
            return Err(FeatureFlagError::InvalidRolloutPercentage(
                rollout_percentage,
            ));
        }

        let now = Utc::now();
        Ok(Self {
            name,
            description,
            enabled: rollout_percentage > 0,
            rollout_percentage,
            created_at: now,
            updated_at: now,
            metadata: serde_json::json!({}),
            strategy: RolloutStrategy::default(),
        })
    }

    /// Create a new flag with a specific rollout strategy
    pub fn with_strategy(
        name: String,
        description: String,
        rollout_percentage: u8,
        strategy: RolloutStrategy,
    ) -> Result<Self, FeatureFlagError> {
        if rollout_percentage > 100 {
            return Err(FeatureFlagError::InvalidRolloutPercentage(
                rollout_percentage,
            ));
        }

        let now = Utc::now();
        Ok(Self {
            name,
            description,
            enabled: rollout_percentage > 0,
            rollout_percentage,
            created_at: now,
            updated_at: now,
            metadata: serde_json::json!({}),
            strategy,
        })
    }

    /// Check if this flag is enabled for a given identifier
    ///
    /// The identifier (e.g., user_id or task_id) is used to ensure
    /// consistent rollout experience - the same identifier will
    /// always get the same result for non-random strategies.
    pub fn is_enabled_for(&self, identifier: &str) -> bool {
        if !self.enabled || self.rollout_percentage == 0 {
            return false;
        }

        if self.rollout_percentage >= 100 {
            return true;
        }

        // Use hash for consistent percentage-based rollout
        let hash = self.hash_identifier(identifier);
        let bucket = (hash % 100) as u8;
        bucket < self.rollout_percentage
    }

    /// Get the rollout percentage
    pub fn rollout_percentage(&self) -> u8 {
        self.rollout_percentage
    }

    /// Check if the flag is globally enabled (regardless of rollout)
    pub fn is_globally_enabled(&self) -> bool {
        self.enabled
    }

    /// Enable the flag (sets enabled to true)
    /// If rollout is currently 0%, sets it to 100% to ensure the flag is actually enabled
    pub fn enable(&mut self) {
        self.enabled = true;
        // If rollout was 0%, set to 100% to make the flag effectively enabled
        if self.rollout_percentage == 0 {
            self.rollout_percentage = 100;
        }
        self.updated_at = Utc::now();
    }

    /// Disable the flag (sets enabled to false)
    pub fn disable(&mut self) {
        self.enabled = false;
        self.updated_at = Utc::now();
    }

    /// Set rollout percentage
    ///
    /// Also automatically enables/disables the flag based on percentage.
    pub fn set_rollout(&mut self, percentage: u8) -> Result<(), FeatureFlagError> {
        if percentage > 100 {
            return Err(FeatureFlagError::InvalidRolloutPercentage(percentage));
        }

        self.rollout_percentage = percentage;
        self.enabled = percentage > 0;
        self.updated_at = Utc::now();
        Ok(())
    }

    /// Set flag metadata
    pub fn set_metadata(&mut self, metadata: serde_json::Value) {
        self.metadata = metadata;
        self.updated_at = Utc::now();
    }

    /// Hash an identifier for consistent bucket assignment
    fn hash_identifier(&self, identifier: &str) -> usize {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        identifier.hash(&mut hasher);
        // Mix in the flag name for per-flag consistency
        self.name.hash(&mut hasher);
        hasher.finish() as usize
    }
}

/// Manager for all feature flags with rollback support
///
/// # Example
///
/// ```rust
/// use swell_orchestrator::FeatureFlagManager;
///
/// let manager = FeatureFlagManager::new();
///
/// // Create flags
/// manager.create_flag("dark_mode", "Dark mode UI", 0).unwrap();
/// manager.create_flag("new_algorithm", "New ranking algorithm", 10).unwrap();
///
/// // Check flags
/// assert!(manager.is_enabled("dark_mode", Some("user123")));
///
/// // Gradual rollout
/// manager.set_rollout("new_algorithm", 50).unwrap();
///
/// // Snapshot and rollback
/// manager.snapshot("new_algorithm").unwrap();
/// manager.set_rollout("new_algorithm", 100).unwrap();
/// manager.rollback("new_algorithm").unwrap(); // Back to 50%
/// ```
#[derive(Debug, Clone)]
pub struct FeatureFlagManager {
    /// All registered feature flags
    flags: HashMap<String, FeatureFlag>,
    /// Snapshots for rollback (flag_name -> snapshots)
    snapshots: HashMap<String, Vec<FlagSnapshot>>,
    /// Configuration
    config: FeatureFlagManagerConfig,
}

/// Configuration for FeatureFlagManager
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureFlagManagerConfig {
    /// Maximum snapshots to keep per flag (0 = unlimited)
    pub max_snapshots_per_flag: usize,
    /// Whether to auto-snapshot before changes
    pub auto_snapshot: bool,
    /// Default rollout percentage for new flags
    pub default_rollout: u8,
}

impl Default for FeatureFlagManagerConfig {
    fn default() -> Self {
        Self {
            max_snapshots_per_flag: 10,
            auto_snapshot: true,
            default_rollout: 0,
        }
    }
}

impl Default for FeatureFlagManager {
    fn default() -> Self {
        Self::new()
    }
}

impl FeatureFlagManager {
    /// Create a new feature flag manager
    pub fn new() -> Self {
        Self {
            flags: HashMap::new(),
            snapshots: HashMap::new(),
            config: FeatureFlagManagerConfig::default(),
        }
    }

    /// Create a new manager with custom configuration
    pub fn with_config(config: FeatureFlagManagerConfig) -> Self {
        Self {
            flags: HashMap::new(),
            snapshots: HashMap::new(),
            config,
        }
    }

    /// Create a new feature flag
    ///
    /// # Errors
    /// Returns `FeatureFlagError::FlagAlreadyExists` if a flag with this name exists
    pub fn create_flag(
        &mut self,
        name: &str,
        description: &str,
        rollout_percentage: u8,
    ) -> Result<FeatureFlag, FeatureFlagError> {
        self.validate_flag_name(name)?;

        if self.flags.contains_key(name) {
            return Err(FeatureFlagError::FlagAlreadyExists(name.to_string()));
        }

        let flag = FeatureFlag::new(
            name.to_string(),
            description.to_string(),
            rollout_percentage,
        )?;

        info!(flag = %name, rollout = %rollout_percentage, "Feature flag created");
        self.flags.insert(name.to_string(), flag.clone());
        Ok(flag)
    }

    /// Create a flag with a specific rollout strategy
    pub fn create_flag_with_strategy(
        &mut self,
        name: &str,
        description: &str,
        rollout_percentage: u8,
        strategy: RolloutStrategy,
    ) -> Result<FeatureFlag, FeatureFlagError> {
        self.validate_flag_name(name)?;

        if self.flags.contains_key(name) {
            return Err(FeatureFlagError::FlagAlreadyExists(name.to_string()));
        }

        let flag = FeatureFlag::with_strategy(
            name.to_string(),
            description.to_string(),
            rollout_percentage,
            strategy.clone(),
        )?;

        info!(flag = %name, rollout = %rollout_percentage, strategy = ?strategy, "Feature flag created with strategy");
        self.flags.insert(name.to_string(), flag.clone());
        Ok(flag)
    }

    /// Get a feature flag by name
    pub fn get_flag(&self, name: &str) -> Result<FeatureFlag, FeatureFlagError> {
        self.flags
            .get(name)
            .cloned()
            .ok_or(FeatureFlagError::FlagNotFound(name.to_string()))
    }

    /// Check if a flag exists
    pub fn has_flag(&self, name: &str) -> bool {
        self.flags.contains_key(name)
    }

    /// List all feature flags
    pub fn list_flags(&self) -> Vec<FeatureFlag> {
        self.flags.values().cloned().collect()
    }

    /// Enable a feature flag
    ///
    /// If `auto_snapshot` is enabled, takes a snapshot before the change.
    pub fn enable(&mut self, name: &str) -> Result<(), FeatureFlagError> {
        if self.config.auto_snapshot {
            self.take_snapshot_internal(name, Some("Before enable".to_string()))?;
        }

        let flag = self
            .flags
            .get_mut(name)
            .ok_or(FeatureFlagError::FlagNotFound(name.to_string()))?;

        flag.enable();
        info!(flag = %name, "Feature flag enabled");
        Ok(())
    }

    /// Disable a feature flag
    ///
    /// This is useful for instant rollback of a feature.
    /// If `auto_snapshot` is enabled, takes a snapshot before the change.
    pub fn disable(&mut self, name: &str) -> Result<(), FeatureFlagError> {
        if self.config.auto_snapshot {
            self.take_snapshot_internal(name, Some("Before disable".to_string()))?;
        }

        let flag = self
            .flags
            .get_mut(name)
            .ok_or(FeatureFlagError::FlagNotFound(name.to_string()))?;

        flag.disable();
        warn!(flag = %name, "Feature flag DISABLED - instant rollback triggered");
        Ok(())
    }

    /// Set the rollout percentage for a flag
    ///
    /// # Arguments
    /// * `name` - Flag name
    /// * `percentage` - New rollout percentage (0-100)
    ///
    /// If `auto_snapshot` is enabled, takes a snapshot before the change.
    pub fn set_rollout(&mut self, name: &str, percentage: u8) -> Result<(), FeatureFlagError> {
        if self.config.auto_snapshot {
            self.take_snapshot_internal(
                name,
                Some(format!("Before rollout change to {}%", percentage)),
            )?;
        }

        let flag = self
            .flags
            .get_mut(name)
            .ok_or(FeatureFlagError::FlagNotFound(name.to_string()))?;

        flag.set_rollout(percentage)?;
        info!(flag = %name, rollout = %percentage, "Feature flag rollout updated");
        Ok(())
    }

    /// Check if a flag is enabled for a given identifier
    ///
    /// The identifier is used for consistent percentage-based rollouts.
    /// If `None` is provided, uses a random identifier.
    pub fn is_enabled(&self, name: &str, identifier: Option<&str>) -> bool {
        let flag = match self.flags.get(name) {
            Some(f) => f,
            None => return false,
        };

        let id = identifier.unwrap_or_else(|| {
            // Generate a random identifier if none provided
            static RANDOM_ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
            RANDOM_ID.get_or_init(|| Uuid::new_v4().to_string())
        });

        flag.is_enabled_for(id)
    }

    /// Take a snapshot of a flag's current state
    ///
    /// Snapshots are stored and can be used for instant rollback.
    /// If `max_snapshots_per_flag` is set, old snapshots are pruned.
    pub fn snapshot(&mut self, name: &str) -> Result<FlagSnapshot, FeatureFlagError> {
        self.take_snapshot_internal(name, None)
    }

    /// Take a snapshot with a description
    pub fn snapshot_with_description(
        &mut self,
        name: &str,
        description: &str,
    ) -> Result<FlagSnapshot, FeatureFlagError> {
        self.take_snapshot_internal(name, Some(description.to_string()))
    }

    /// Internal snapshot implementation
    fn take_snapshot_internal(
        &mut self,
        name: &str,
        description: Option<String>,
    ) -> Result<FlagSnapshot, FeatureFlagError> {
        let flag = self
            .flags
            .get(name)
            .ok_or(FeatureFlagError::FlagNotFound(name.to_string()))?;

        let snapshot = FlagSnapshot::new(flag, description);

        // Store snapshot
        let snapshots = self.snapshots.entry(name.to_string()).or_default();

        // Prune if necessary
        if self.config.max_snapshots_per_flag > 0
            && snapshots.len() >= self.config.max_snapshots_per_flag
        {
            // Remove oldest snapshot
            snapshots.remove(0);
        }

        snapshots.push(snapshot.clone());
        info!(
            flag = %name,
            snapshot_id = %snapshot.id,
            snapshot_count = snapshots.len(),
            "Feature flag snapshot taken"
        );

        Ok(snapshot)
    }

    /// Rollback a flag to its most recent snapshot
    ///
    /// Returns the flag state before rollback for reference.
    pub fn rollback(&mut self, name: &str) -> Result<FeatureFlag, FeatureFlagError> {
        // Get current state to return
        let current = self.get_flag(name)?;

        let snapshots = self
            .snapshots
            .get_mut(name)
            .ok_or(FeatureFlagError::NoSnapshot(name.to_string()))?;

        let snapshot = snapshots
            .pop()
            .ok_or(FeatureFlagError::NoSnapshot(name.to_string()))?;

        // Restore the flag state
        if let Some(flag) = self.flags.get_mut(name) {
            *flag = snapshot.flag_state.clone();
            flag.updated_at = Utc::now();
        }

        warn!(
            flag = %name,
            snapshot_id = %snapshot.id,
            remaining_snapshots = snapshots.len(),
            "Feature flag rolled back to snapshot"
        );

        Ok(current)
    }

    /// Rollback to a specific snapshot by ID
    pub fn rollback_to(
        &mut self,
        name: &str,
        snapshot_id: Uuid,
    ) -> Result<FeatureFlag, FeatureFlagError> {
        // Get current state to return
        let current = self.get_flag(name)?;

        let snapshots = self
            .snapshots
            .get_mut(name)
            .ok_or(FeatureFlagError::NoSnapshot(name.to_string()))?;

        // Find the snapshot
        let idx = snapshots
            .iter()
            .position(|s| s.id == snapshot_id)
            .ok_or_else(|| FeatureFlagError::NoSnapshot(name.to_string()))?;

        let snapshot = snapshots.remove(idx);

        // Restore the flag state
        if let Some(flag) = self.flags.get_mut(name) {
            *flag = snapshot.flag_state.clone();
            flag.updated_at = Utc::now();
        }

        warn!(
            flag = %name,
            snapshot_id = %snapshot_id,
            remaining_snapshots = snapshots.len(),
            "Feature flag rolled back to specific snapshot"
        );

        Ok(current)
    }

    /// Get all snapshots for a flag
    pub fn get_snapshots(&self, name: &str) -> Result<Vec<FlagSnapshot>, FeatureFlagError> {
        self.snapshots
            .get(name)
            .cloned()
            .ok_or(FeatureFlagError::NoSnapshot(name.to_string()))
    }

    /// Get the latest snapshot for a flag
    pub fn get_latest_snapshot(&self, name: &str) -> Result<FlagSnapshot, FeatureFlagError> {
        self.snapshots
            .get(name)
            .and_then(|s| s.last().cloned())
            .ok_or(FeatureFlagError::NoSnapshot(name.to_string()))
    }

    /// Delete a feature flag
    pub fn delete_flag(&mut self, name: &str) -> Result<(), FeatureFlagError> {
        if self.flags.remove(name).is_none() {
            return Err(FeatureFlagError::FlagNotFound(name.to_string()));
        }

        // Also remove snapshots
        self.snapshots.remove(name);

        info!(flag = %name, "Feature flag deleted");
        Ok(())
    }

    /// Validate flag name
    fn validate_flag_name(&self, name: &str) -> Result<(), FeatureFlagError> {
        if name.is_empty() {
            return Err(FeatureFlagError::InvalidFlagName(
                "Flag name cannot be empty".to_string(),
            ));
        }

        if name.len() > 64 {
            return Err(FeatureFlagError::InvalidFlagName(
                "Flag name cannot exceed 64 characters".to_string(),
            ));
        }

        // Allow alphanumeric, underscores, and hyphens
        if !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Err(FeatureFlagError::InvalidFlagName(
                "Flag name can only contain alphanumeric characters, underscores, and hyphens"
                    .to_string(),
            ));
        }

        Ok(())
    }

    /// Get manager configuration
    pub fn config(&self) -> &FeatureFlagManagerConfig {
        &self.config
    }

    /// Update manager configuration
    pub fn set_config(&mut self, config: FeatureFlagManagerConfig) {
        self.config = config;
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- FeatureFlag Tests ---

    #[test]
    fn test_feature_flag_creation() {
        let flag = FeatureFlag::new("test_flag".to_string(), "Test".to_string(), 50).unwrap();
        assert_eq!(flag.name, "test_flag");
        assert_eq!(flag.rollout_percentage(), 50);
        assert!(flag.is_globally_enabled());
    }

    #[test]
    fn test_feature_flag_creation_invalid_rollout() {
        let result = FeatureFlag::new("test".to_string(), "Test".to_string(), 150);
        assert!(matches!(
            result.unwrap_err(),
            FeatureFlagError::InvalidRolloutPercentage(150)
        ));
    }

    #[test]
    fn test_feature_flag_enable_disable() {
        let mut flag = FeatureFlag::new("test".to_string(), "Test".to_string(), 100).unwrap();
        assert!(flag.is_globally_enabled());

        flag.disable();
        assert!(!flag.is_globally_enabled());

        flag.enable();
        assert!(flag.is_globally_enabled());
    }

    #[test]
    fn test_feature_flag_rollout_0_disabled() {
        let mut flag = FeatureFlag::new("test".to_string(), "Test".to_string(), 0).unwrap();
        assert!(!flag.is_globally_enabled());
        assert!(!flag.is_enabled_for("user123"));

        flag.set_rollout(100).unwrap();
        assert!(flag.is_enabled_for("user123"));
    }

    #[test]
    fn test_feature_flag_rollout_100_always_enabled() {
        let flag = FeatureFlag::new("test".to_string(), "Test".to_string(), 100).unwrap();
        assert!(flag.is_enabled_for("any_user"));
        assert!(flag.is_enabled_for("another_user"));
    }

    #[test]
    fn test_feature_flag_consistent_rollout() {
        let flag = FeatureFlag::new("test".to_string(), "Test".to_string(), 50).unwrap();

        // Same user should always get the same result
        let result1 = flag.is_enabled_for("user123");
        let result2 = flag.is_enabled_for("user123");
        assert_eq!(result1, result2);

        // Different users may get different results (statistically ~50/50)
    }

    #[test]
    fn test_feature_flag_set_rollout() {
        let mut flag = FeatureFlag::new("test".to_string(), "Test".to_string(), 0).unwrap();

        flag.set_rollout(25).unwrap();
        assert_eq!(flag.rollout_percentage(), 25);
        assert!(flag.is_globally_enabled()); // 25 > 0

        flag.set_rollout(0).unwrap();
        assert_eq!(flag.rollout_percentage(), 0);
        assert!(!flag.is_globally_enabled());
    }

    // --- FeatureFlagManager Tests ---

    #[test]
    fn test_manager_create_flag() {
        let mut manager = FeatureFlagManager::new();

        let flag = manager.create_flag("dark_mode", "Dark mode UI", 0).unwrap();

        assert_eq!(flag.name, "dark_mode");
        assert!(manager.has_flag("dark_mode"));
    }

    #[test]
    fn test_manager_create_duplicate_flag() {
        let mut manager = FeatureFlagManager::new();
        manager.create_flag("test", "Test", 0).unwrap();

        let result = manager.create_flag("test", "Test 2", 0);
        assert!(matches!(
            result.unwrap_err(),
            FeatureFlagError::FlagAlreadyExists(_)
        ));
    }

    #[test]
    fn test_manager_get_flag() {
        let mut manager = FeatureFlagManager::new();
        manager.create_flag("test", "Test", 50).unwrap();

        let flag = manager.get_flag("test").unwrap();
        assert_eq!(flag.rollout_percentage(), 50);
    }

    #[test]
    fn test_manager_get_nonexistent_flag() {
        let manager = FeatureFlagManager::new();
        let result = manager.get_flag("nonexistent");
        assert!(matches!(
            result.unwrap_err(),
            FeatureFlagError::FlagNotFound(_)
        ));
    }

    #[test]
    fn test_manager_enable_flag() {
        let mut manager = FeatureFlagManager::new();
        manager.create_flag("test", "Test", 0).unwrap();

        assert!(!manager.is_enabled("test", None));

        manager.enable("test").unwrap();

        assert!(manager.is_enabled("test", None));
    }

    #[test]
    fn test_manager_disable_flag() {
        let mut manager = FeatureFlagManager::new();
        manager.create_flag("test", "Test", 100).unwrap();

        assert!(manager.is_enabled("test", None));

        manager.disable("test").unwrap();

        assert!(!manager.is_enabled("test", None));
    }

    #[test]
    fn test_manager_set_rollout() {
        let mut manager = FeatureFlagManager::new();
        manager.create_flag("test", "Test", 0).unwrap();

        manager.set_rollout("test", 75).unwrap();

        let flag = manager.get_flag("test").unwrap();
        assert_eq!(flag.rollout_percentage(), 75);
    }

    #[test]
    fn test_manager_snapshot_and_rollback() {
        let mut manager = FeatureFlagManager::new();
        manager.create_flag("test", "Test", 0).unwrap();

        // Take a snapshot
        let snapshot = manager.snapshot("test").unwrap();
        assert_eq!(snapshot.flag_name, "test");
        assert_eq!(snapshot.flag_state.rollout_percentage(), 0);

        // Change the flag
        manager.set_rollout("test", 100).unwrap();
        assert_eq!(manager.get_flag("test").unwrap().rollout_percentage(), 100);

        // Rollback
        let old_state = manager.rollback("test").unwrap();
        assert_eq!(old_state.rollout_percentage(), 100); // Returns state before rollback

        assert_eq!(manager.get_flag("test").unwrap().rollout_percentage(), 0);
    }

    #[test]
    fn test_manager_rollback_without_snapshot() {
        let mut manager = FeatureFlagManager::new();
        manager.create_flag("test", "Test", 0).unwrap();

        let result = manager.rollback("test");
        assert!(matches!(
            result.unwrap_err(),
            FeatureFlagError::NoSnapshot(_)
        ));
    }

    #[test]
    fn test_manager_snapshot_with_description() {
        let mut manager = FeatureFlagManager::new();
        manager.create_flag("test", "Test", 0).unwrap();

        let snapshot = manager
            .snapshot_with_description("test", "Before production launch")
            .unwrap();

        assert_eq!(
            snapshot.description,
            Some("Before production launch".to_string())
        );
    }

    #[test]
    fn test_manager_multiple_snapshots() {
        let mut manager = FeatureFlagManager::new();
        manager.set_config(FeatureFlagManagerConfig {
            max_snapshots_per_flag: 3,
            auto_snapshot: false,
            default_rollout: 0,
        });

        manager.create_flag("test", "Test", 0).unwrap();

        // Take 4 manual snapshots - with max_snapshots_per_flag = 3,
        // the first one should be pruned when the 4th is added
        let snap1 = manager.snapshot("test").unwrap();
        manager.set_rollout("test", 25).unwrap();
        let snap2 = manager.snapshot("test").unwrap();
        manager.set_rollout("test", 50).unwrap();
        let snap3 = manager.snapshot("test").unwrap();
        manager.set_rollout("test", 75).unwrap();
        let snap4 = manager.snapshot("test").unwrap();

        // 4 snapshots taken, but max is 3, so first one should be pruned
        let snapshots = manager.get_snapshots("test").unwrap();
        assert_eq!(snapshots.len(), 3);

        // snap1 should be removed (pruned), snap2, snap3, snap4 should remain
        assert!(!snapshots.iter().any(|s| s.id == snap1.id));
        assert!(snapshots.iter().any(|s| s.id == snap2.id));
        assert!(snapshots.iter().any(|s| s.id == snap3.id));
        assert!(snapshots.iter().any(|s| s.id == snap4.id));
    }

    #[test]
    fn test_manager_rollback_to_specific_snapshot() {
        let mut manager = FeatureFlagManager::new();
        manager.create_flag("test", "Test", 0).unwrap();

        let snap1 = manager.snapshot("test").unwrap();
        manager.set_rollout("test", 50).unwrap();
        let snap2 = manager.snapshot("test").unwrap();
        manager.set_rollout("test", 100).unwrap();

        // Rollback to snap1 (which has 0% rollout)
        manager.rollback_to("test", snap1.id).unwrap();

        assert_eq!(manager.get_flag("test").unwrap().rollout_percentage(), 0);

        // Rollback to snap2 (which has 50% rollout)
        manager.rollback_to("test", snap2.id).unwrap();

        assert_eq!(manager.get_flag("test").unwrap().rollout_percentage(), 50);
    }

    #[test]
    fn test_manager_list_flags() {
        let mut manager = FeatureFlagManager::new();
        manager.create_flag("flag1", "Flag 1", 0).unwrap();
        manager.create_flag("flag2", "Flag 2", 50).unwrap();
        manager.create_flag("flag3", "Flag 3", 100).unwrap();

        let flags = manager.list_flags();
        assert_eq!(flags.len(), 3);
    }

    #[test]
    fn test_manager_delete_flag() {
        let mut manager = FeatureFlagManager::new();
        manager.create_flag("test", "Test", 0).unwrap();
        manager.snapshot("test").unwrap();

        manager.delete_flag("test").unwrap();

        assert!(!manager.has_flag("test"));
        assert!(manager.get_snapshots("test").is_err());
    }

    #[test]
    fn test_manager_invalid_flag_names() {
        let mut manager = FeatureFlagManager::new();

        // Empty name
        let result = manager.create_flag("", "Test", 0);
        assert!(matches!(
            result.unwrap_err(),
            FeatureFlagError::InvalidFlagName(_)
        ));

        // Name with invalid characters
        let result = manager.create_flag("invalid name!", "Test", 0);
        assert!(matches!(
            result.unwrap_err(),
            FeatureFlagError::InvalidFlagName(_)
        ));

        // Name too long
        let long_name = "a".repeat(65);
        let result = manager.create_flag(&long_name, "Test", 0);
        assert!(matches!(
            result.unwrap_err(),
            FeatureFlagError::InvalidFlagName(_)
        ));
    }

    #[test]
    fn test_manager_consistent_rollout_across_checks() {
        let mut manager = FeatureFlagManager::new();
        manager.create_flag("test", "Test", 50).unwrap();

        let user_id = "user_12345";

        // Multiple checks should return the same result
        assert_eq!(
            manager.is_enabled("test", Some(user_id)),
            manager.is_enabled("test", Some(user_id))
        );
        assert_eq!(
            manager.is_enabled("test", Some(user_id)),
            manager.is_enabled("test", Some(user_id))
        );
    }

    #[test]
    fn test_manager_different_users_different_rollout() {
        let mut manager = FeatureFlagManager::new();
        // Set to exactly 50% rollout
        manager.create_flag("test", "Test", 50).unwrap();

        // With a proper hash, we should see roughly 50/50 split
        // across a large sample of users
        let mut enabled_count = 0;
        let total_users = 1000;

        for i in 0..total_users {
            let user_id = format!("user_{}", i);
            if manager.is_enabled("test", Some(&user_id)) {
                enabled_count += 1;
            }
        }

        // Should be roughly 50% (allow 10% tolerance)
        let percentage = (enabled_count as f64 / total_users as f64) * 100.0;
        assert!(
            percentage > 40.0 && percentage < 60.0,
            "Expected ~50% enabled, got {}%",
            percentage
        );
    }

    // --- RolloutStrategy Tests ---

    #[test]
    fn test_phased_rollout_strategy() {
        let strategy = RolloutStrategy::Phased {
            phases: vec![
                RolloutPhase {
                    percentage: 10,
                    min_iterations: 0,
                },
                RolloutPhase {
                    percentage: 50,
                    min_iterations: 2,
                },
                RolloutPhase {
                    percentage: 100,
                    min_iterations: 5,
                },
            ],
        };

        // Use 100% rollout to ensure flag is enabled regardless of hash bucket
        let flag = FeatureFlag::with_strategy(
            "phased_test".to_string(),
            "Phased rollout test".to_string(),
            100,
            strategy,
        )
        .unwrap();

        // Flag should be enabled with 100% rollout
        assert!(flag.is_enabled_for("user_1"));
        assert!(flag.is_enabled_for("any_user"));
    }

    // --- FeatureFlagManagerConfig Tests ---

    #[test]
    fn test_manager_config_defaults() {
        let config = FeatureFlagManagerConfig::default();
        assert_eq!(config.max_snapshots_per_flag, 10);
        assert!(config.auto_snapshot);
        assert_eq!(config.default_rollout, 0);
    }

    #[test]
    fn test_manager_with_custom_config() {
        let config = FeatureFlagManagerConfig {
            max_snapshots_per_flag: 5,
            auto_snapshot: false,
            default_rollout: 100,
        };

        let manager = FeatureFlagManager::with_config(config.clone());
        assert_eq!(manager.config().max_snapshots_per_flag, 5);
        assert!(!manager.config().auto_snapshot);
    }

    #[test]
    fn test_manager_auto_snapshot_disabled() {
        let config = FeatureFlagManagerConfig {
            max_snapshots_per_flag: 10,
            auto_snapshot: false,
            default_rollout: 0,
        };

        let mut manager = FeatureFlagManager::with_config(config);

        // Manually create and snapshot
        manager.create_flag("test", "Test", 0).unwrap();
        manager.snapshot("test").unwrap();

        // Change flag without auto-snapshot
        manager.set_rollout("test", 50).unwrap();

        // Should still have only 1 snapshot (the manual one)
        let snapshots = manager.get_snapshots("test").unwrap();
        assert_eq!(snapshots.len(), 1);
    }

    #[test]
    fn test_manager_no_auto_snapshot_rollback() {
        let config = FeatureFlagManagerConfig {
            max_snapshots_per_flag: 10,
            auto_snapshot: false,
            default_rollout: 0,
        };

        let mut manager = FeatureFlagManager::with_config(config);
        manager.create_flag("test", "Test", 0).unwrap();

        // No snapshots initially
        assert!(manager.get_snapshots("test").is_err());

        // Rollback without snapshots should fail
        let result = manager.rollback("test");
        assert!(matches!(
            result.unwrap_err(),
            FeatureFlagError::NoSnapshot(_)
        ));
    }
}
