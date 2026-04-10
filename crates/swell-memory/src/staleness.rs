// staleness.rs - Memory staleness detection and reinforcement tracking
//
// This module provides functionality to:
// - Track last_reinforcement timestamp for memories
// - Detect when memories become stale (not reinforced within configured window)
// - Exclude stale memories from retrieval
// - Reinforce memories when they are accessed or used
//
// Staleness is different from decay:
// - Decay: Gradual reduction in retrieval score over time
// - Staleness: Binary state - memory is either stale or not, based on time since reinforcement

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// Default staleness window in days
pub const DEFAULT_STALENESS_WINDOW_DAYS: i64 = 30;

/// Default reinforcement update interval in days
pub const DEFAULT_REINFORCEMENT_INTERVAL_DAYS: i64 = 7;

/// Configuration for staleness detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StalenessConfig {
    /// Window in days after which a memory becomes stale if not reinforced
    pub staleness_window_days: i64,
    /// How often to update last_reinforcement when memory is accessed (days)
    pub reinforcement_update_interval_days: i64,
}

impl Default for StalenessConfig {
    fn default() -> Self {
        Self {
            staleness_window_days: DEFAULT_STALENESS_WINDOW_DAYS,
            reinforcement_update_interval_days: DEFAULT_REINFORCEMENT_INTERVAL_DAYS,
        }
    }
}

impl StalenessConfig {
    /// Create a new staleness config with custom values
    pub fn new(staleness_window_days: i64, reinforcement_update_interval_days: i64) -> Self {
        Self {
            staleness_window_days,
            reinforcement_update_interval_days,
        }
    }

    /// Calculate the staleness threshold datetime
    pub fn staleness_threshold(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        now - Duration::days(self.staleness_window_days)
    }

    /// Check if a memory should be reinforced based on last_reinforcement
    pub fn should_reinforce(&self, last_reinforcement: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
        match last_reinforcement {
            None => true, // Never reinforced = should reinforce
            Some(lr) => {
                let days_since = (now - lr).num_days();
                days_since >= self.reinforcement_update_interval_days
            }
        }
    }
}

/// Result of staleness check
#[derive(Debug, Clone)]
pub struct StalenessCheckResult {
    /// Whether the memory is stale
    pub is_stale: bool,
    /// Days since last reinforcement
    pub days_since_reinforcement: i64,
    /// The staleness threshold used
    pub threshold_days: i64,
    /// Whether the memory should be reinforced on next access
    pub should_reinforce: bool,
    /// Time until the memory becomes stale (if not stale yet)
    pub days_until_stale: Option<i64>,
}

impl StalenessCheckResult {
    /// Create a result for a fresh memory (never reinforced)
    pub fn fresh() -> Self {
        Self {
            is_stale: false,
            days_since_reinforcement: 0,
            threshold_days: DEFAULT_STALENESS_WINDOW_DAYS,
            should_reinforce: true,
            days_until_stale: Some(DEFAULT_STALENESS_WINDOW_DAYS),
        }
    }

    /// Create a result for a memory that just became stale
    pub fn newly_stale(days_since_reinforcement: i64, threshold_days: i64) -> Self {
        Self {
            is_stale: true,
            days_since_reinforcement,
            threshold_days,
            should_reinforce: false,
            days_until_stale: None,
        }
    }

    /// Create a result for a memory that is not stale
    pub fn not_stale(days_since_reinforcement: i64, threshold_days: i64) -> Self {
        Self {
            is_stale: false,
            days_since_reinforcement,
            threshold_days,
            should_reinforce: days_since_reinforcement >= DEFAULT_REINFORCEMENT_INTERVAL_DAYS,
            days_until_stale: Some(threshold_days - days_since_reinforcement),
        }
    }
}

/// Check staleness of a memory
pub fn check_staleness(
    last_reinforcement: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    config: &StalenessConfig,
) -> StalenessCheckResult {
    match last_reinforcement {
        None => {
            // Never reinforced - check against created_at would be better but we don't have it here
            // For now, treat as fresh but needing reinforcement
            StalenessCheckResult::fresh()
        }
        Some(lr) => {
            let days_since = (now - lr).num_days();
            let threshold = config.staleness_window_days;

            if days_since >= threshold {
                StalenessCheckResult::newly_stale(days_since, threshold)
            } else {
                StalenessCheckResult::not_stale(days_since, threshold)
            }
        }
    }
}

/// Check if a memory is stale based on its last_reinforcement timestamp
pub fn is_stale_memory(
    last_reinforcement: Option<DateTime<Utc>>,
    config: &StalenessConfig,
) -> bool {
    check_staleness(last_reinforcement, Utc::now(), config).is_stale
}

/// Calculate days until a memory becomes stale
pub fn days_until_stale(
    last_reinforcement: Option<DateTime<Utc>>,
    config: &StalenessConfig,
) -> Option<i64> {
    check_staleness(last_reinforcement, Utc::now(), config).days_until_stale
}

/// Reinforce a memory - update its last_reinforcement timestamp
pub fn reinforce(last_reinforcement: &mut Option<DateTime<Utc>>, now: DateTime<Utc>) {
    *last_reinforcement = Some(now);
}

/// Check staleness using default config
pub fn check_staleness_default(last_reinforcement: Option<DateTime<Utc>>) -> StalenessCheckResult {
    check_staleness(last_reinforcement, Utc::now(), &StalenessConfig::default())
}

/// Check if stale using default config
pub fn is_stale_default(last_reinforcement: Option<DateTime<Utc>>) -> bool {
    is_stale_memory(last_reinforcement, &StalenessConfig::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_staleness_config_default() {
        let config = StalenessConfig::default();
        assert_eq!(config.staleness_window_days, 30);
        assert_eq!(config.reinforcement_update_interval_days, 7);
    }

    #[test]
    fn test_staleness_config_custom() {
        let config = StalenessConfig::new(60, 14);
        assert_eq!(config.staleness_window_days, 60);
        assert_eq!(config.reinforcement_update_interval_days, 14);
    }

    #[test]
    fn test_staleness_threshold() {
        let config = StalenessConfig::default();
        let now = Utc::now();
        let threshold = config.staleness_threshold(now);

        let expected = now - Duration::days(30);
        assert_eq!(threshold, expected);
    }

    #[test]
    fn test_check_staleness_never_reinforced() {
        let config = StalenessConfig::default();
        let now = Utc::now();

        let result = check_staleness(None, now, &config);

        assert!(!result.is_stale);
        assert!(result.should_reinforce);
        assert_eq!(result.days_since_reinforcement, 0);
    }

    #[test]
    fn test_check_staleness_fresh_memory() {
        let config = StalenessConfig::default();
        let now = Utc::now();
        let yesterday = now - Duration::days(1);

        let result = check_staleness(Some(yesterday), now, &config);

        assert!(!result.is_stale);
        assert!(!result.should_reinforce); // Only 1 day since reinforcement
        assert_eq!(result.days_since_reinforcement, 1);
        assert_eq!(result.days_until_stale, Some(29));
    }

    #[test]
    fn test_check_staleness_needing_reinforcement() {
        let config = StalenessConfig::default();
        let now = Utc::now();
        let week_ago = now - Duration::days(8); // More than 7 days = should reinforce

        let result = check_staleness(Some(week_ago), now, &config);

        assert!(!result.is_stale);
        assert!(result.should_reinforce); // 8 days > 7 day interval
        assert_eq!(result.days_since_reinforcement, 8);
    }

    #[test]
    fn test_check_staleness_becoming_stale() {
        let config = StalenessConfig::default();
        let now = Utc::now();
        let exactly_30_days_ago = now - Duration::days(30);

        let result = check_staleness(Some(exactly_30_days_ago), now, &config);

        assert!(result.is_stale); // Exactly at threshold = stale
        assert_eq!(result.days_since_reinforcement, 30);
        assert!(result.days_until_stale.is_none());
    }

    #[test]
    fn test_check_staleness_long_stale() {
        let config = StalenessConfig::default();
        let now = Utc::now();
        let months_ago = now - Duration::days(90);

        let result = check_staleness(Some(months_ago), now, &config);

        assert!(result.is_stale);
        assert_eq!(result.days_since_reinforcement, 90);
        assert!(result.days_until_stale.is_none());
    }

    #[test]
    fn test_is_stale_memory() {
        let config = StalenessConfig::default();
        let now = Utc::now();

        // Never reinforced - not stale (fresh)
        assert!(!is_stale_memory(None, &config));

        // Recent reinforcement - not stale
        let yesterday = now - Duration::days(1);
        assert!(!is_stale_memory(Some(yesterday), &config));

        // Old reinforcement - stale
        let months_ago = now - Duration::days(60);
        assert!(is_stale_memory(Some(months_ago), &config));
    }

    #[test]
    fn test_reinforce() {
        let now = Utc::now();
        let mut last_reinforcement: Option<DateTime<Utc>> = None;

        // First reinforcement
        reinforce(&mut last_reinforcement, now);
        assert!(last_reinforcement.is_some());
        assert_eq!(last_reinforcement.unwrap(), now);

        // Second reinforcement (later)
        let later = now + Duration::hours(1);
        reinforce(&mut last_reinforcement, later);
        assert_eq!(last_reinforcement.unwrap(), later);
    }

    #[test]
    fn test_days_until_stale() {
        let config = StalenessConfig::default();
        let now = Utc::now();

        // Never reinforced - 30 days until stale
        assert_eq!(days_until_stale(None, &config), Some(30));

        // 10 days ago - 20 days until stale
        let ten_days_ago = now - Duration::days(10);
        assert_eq!(days_until_stale(Some(ten_days_ago), &config), Some(20));

        // 45 days ago - already stale
        let forty_five_days_ago = now - Duration::days(45);
        assert_eq!(days_until_stale(Some(forty_five_days_ago), &config), None);
    }

    #[test]
    fn test_staleness_check_result_fresh() {
        let result = StalenessCheckResult::fresh();

        assert!(!result.is_stale);
        assert_eq!(result.days_since_reinforcement, 0);
        assert_eq!(result.threshold_days, 30);
        assert!(result.should_reinforce);
        assert_eq!(result.days_until_stale, Some(30));
    }

    #[test]
    fn test_staleness_check_result_not_stale() {
        let result = StalenessCheckResult::not_stale(10, 30);

        assert!(!result.is_stale);
        assert_eq!(result.days_since_reinforcement, 10);
        assert_eq!(result.threshold_days, 30);
        // 10 days >= 7 day reinforcement interval, so should reinforce
        assert!(result.should_reinforce);
        assert_eq!(result.days_until_stale, Some(20));
    }

    #[test]
    fn test_staleness_check_result_newly_stale() {
        let result = StalenessCheckResult::newly_stale(45, 30);

        assert!(result.is_stale);
        assert_eq!(result.days_since_reinforcement, 45);
        assert_eq!(result.threshold_days, 30);
        assert!(!result.should_reinforce);
        assert!(result.days_until_stale.is_none());
    }

    #[test]
    fn test_should_reinforce() {
        let config = StalenessConfig::default();
        let now = Utc::now();

        // Never reinforced
        assert!(config.should_reinforce(None, now));

        // 5 days ago - less than 7 day interval
        let five_days_ago = now - Duration::days(5);
        assert!(!config.should_reinforce(Some(five_days_ago), now));

        // 7 days ago - exactly at interval
        let seven_days_ago = now - Duration::days(7);
        assert!(config.should_reinforce(Some(seven_days_ago), now));

        // 10 days ago - beyond interval
        let ten_days_ago = now - Duration::days(10);
        assert!(config.should_reinforce(Some(ten_days_ago), now));
    }

    #[test]
    fn test_custom_config_staleness() {
        let config = StalenessConfig::new(7, 1); // 7 days staleness, 1 day reinforcement
        let now = Utc::now();

        // 5 days ago - not stale (threshold is 7)
        let five_days_ago = now - Duration::days(5);
        let result = check_staleness(Some(five_days_ago), now, &config);
        assert!(!result.is_stale);

        // 8 days ago - stale
        let eight_days_ago = now - Duration::days(8);
        let result = check_staleness(Some(eight_days_ago), now, &config);
        assert!(result.is_stale);

        // 2 days ago - should reinforce (1 day interval)
        let two_days_ago = now - Duration::days(2);
        assert!(config.should_reinforce(Some(two_days_ago), now));
    }

    #[test]
    fn test_check_staleness_default() {
        let now = Utc::now();
        let week_ago = now - Duration::days(7);

        let result = check_staleness_default(Some(week_ago));

        // 7 days ago with default 30 day window = not stale but should reinforce
        assert!(!result.is_stale);
        assert!(result.should_reinforce);
    }
}
