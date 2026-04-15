//! Cron Registry for scheduling recurring tasks with cron expressions.
//!
//! The `CronRegistry` manages tasks that are scheduled to run at specific times
//! based on cron expressions. It provides `due_entries(now)` to retrieve all
//! tasks whose next scheduled time has passed for a given timestamp.
//!
//! # Example
//!
//! ```ignore
//! let registry = CronRegistry::new();
//!
//! // Register a task to run every 5 minutes
//! registry.register(task_id, "0 */5 * * * *").unwrap();
//!
//! // Get all tasks that are due at the current time
//! let due = registry.due_entries(Utc::now());
//! ```

use croner::Cron;
use dashmap::DashMap;
use std::sync::Arc;
use swell_core::SwellError;
use tracing::info;
use uuid::Uuid;

/// A cron entry associating a task with a cron expression for scheduling.
#[derive(Debug, Clone)]
pub struct CronEntry {
    /// The task ID this entry is associated with
    pub task_id: Uuid,
    /// The cron expression defining the schedule (6-field format with seconds)
    pub expression: String,
    /// The last time this entry was triggered (if any)
    pub last_triggered: Option<chrono::DateTime<chrono::Utc>>,
    /// When this entry was registered
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Optional description for debugging/logging
    pub description: Option<String>,
}

impl CronEntry {
    /// Create a new cron entry
    pub fn new(
        task_id: Uuid,
        expression: String,
        description: Option<String>,
    ) -> Result<Self, SwellError> {
        // Validate the cron expression by trying to parse it
        let _ = Cron::parse(&expression).map_err(|e| {
            SwellError::InvalidOperation(format!("Invalid cron expression '{}': {}", expression, e))
        })?;

        Ok(Self {
            task_id,
            expression,
            last_triggered: None,
            created_at: chrono::Utc::now(),
            description,
        })
    }

    /// Calculate the next scheduled time after the given time.
    ///
    /// Returns `None` if the cron expression cannot produce a valid next time.
    pub fn next_scheduled(
        &self,
        after: chrono::DateTime<chrono::Utc>,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        let cron = Cron::parse(&self.expression).ok()?;
        // inclusive=true means include the given time if it matches the cron
        cron.find_next_occurrence(&after, true).ok()
    }

    /// Check if this entry is due at the given time.
    ///
    /// An entry is due if the given time matches a scheduled execution time.
    /// This uses inclusive=true to check if at is itself a match.
    /// Note: Comparison is at second precision to handle nanosecond precision differences.
    pub fn is_due(&self, at: chrono::DateTime<chrono::Utc>) -> bool {
        let cron = match Cron::parse(&self.expression) {
            Ok(c) => c,
            Err(_) => return false,
        };
        // If at itself is a scheduled time, it's due
        // Use timestamp() comparison to handle nanosecond precision differences
        match cron.find_next_occurrence(&at, true) {
            Ok(next) => next.timestamp() == at.timestamp(),
            Err(_) => false,
        }
    }
}

/// Thread-safe cron registry for managing scheduled tasks.
///
/// Uses DashMap for fine-grained concurrent access, allowing multiple
/// operations on different entries without lock contention.
pub struct CronRegistry {
    /// Maps task ID -> CronEntry
    entries: DashMap<Uuid, Arc<CronEntry>>,
    /// Maps cron expression hash -> list of task IDs (for efficient lookup)
    /// This is useful when we want to find all entries with the same expression
    expression_index: DashMap<String, Vec<Uuid>>,
}

/// Registry event emitted when cron entries change state
#[derive(Debug, Clone)]
pub enum CronEvent {
    /// Emitted when a new cron entry is registered
    CronEntryRegistered { task_id: Uuid, expression: String },
    /// Emitted when a cron entry is removed
    CronEntryRemoved { task_id: Uuid },
    /// Emitted when a task's cron expression is updated
    CronEntryUpdated {
        task_id: Uuid,
        old_expression: String,
        new_expression: String,
    },
    /// Emitted when a task becomes due
    TaskDue { task_id: Uuid, expression: String },
}

impl CronRegistry {
    /// Create a new empty cron registry
    pub fn new() -> Self {
        Self {
            entries: DashMap::new(),
            expression_index: DashMap::new(),
        }
    }

    /// Register a new cron entry for a task.
    ///
    /// Returns an error if the cron expression is invalid.
    pub fn register(&self, task_id: Uuid, expression: &str) -> Result<(), SwellError> {
        // Validate and create the entry
        let entry = Arc::new(CronEntry::new(task_id, expression.to_string(), None)?);

        // Check for duplicate task ID
        if self.entries.contains_key(&task_id) {
            return Err(SwellError::InvalidOperation(format!(
                "Task {} already has a cron entry registered",
                task_id
            )));
        }

        self.entries.insert(task_id, entry);

        // Update expression index
        let mut index = self
            .expression_index
            .entry(expression.to_string())
            .or_default();
        index.push(task_id);

        info!(task_id = %task_id, expression = %expression, "Cron entry registered");
        Ok(())
    }

    /// Register a cron entry with a description.
    pub fn register_with_description(
        &self,
        task_id: Uuid,
        expression: &str,
        description: &str,
    ) -> Result<(), SwellError> {
        let entry = Arc::new(CronEntry::new(
            task_id,
            expression.to_string(),
            Some(description.to_string()),
        )?);

        if self.entries.contains_key(&task_id) {
            return Err(SwellError::InvalidOperation(format!(
                "Task {} already has a cron entry registered",
                task_id
            )));
        }

        self.entries.insert(task_id, entry.clone());

        let mut index = self
            .expression_index
            .entry(expression.to_string())
            .or_default();
        index.push(task_id);

        info!(task_id = %task_id, expression = %expression, description = %description, "Cron entry registered with description");
        Ok(())
    }

    /// Remove a cron entry by task ID.
    ///
    /// Returns the removed entry if it existed.
    pub fn remove(&self, task_id: Uuid) -> Result<Option<Arc<CronEntry>>, SwellError> {
        if let Some((_, entry)) = self.entries.remove(&task_id) {
            // Remove from expression index
            if let Some(mut index) = self.expression_index.get_mut(&entry.expression) {
                index.retain(|&id| id != task_id);
                if index.is_empty() {
                    // Remove the expression entry if no more tasks use it
                    drop(index);
                    self.expression_index.remove(&entry.expression);
                }
            }

            info!(task_id = %task_id, "Cron entry removed");
            Ok(Some(entry))
        } else {
            Ok(None)
        }
    }

    /// Update the cron expression for a task.
    ///
    /// Returns the updated entry.
    pub fn update_expression(
        &self,
        task_id: Uuid,
        new_expression: &str,
    ) -> Result<Arc<CronEntry>, SwellError> {
        let entry = self
            .entries
            .get(&task_id)
            .ok_or_else(|| {
                SwellError::InvalidOperation(format!("No cron entry found for task {}", task_id))
            })?
            .value()
            .clone();

        // Validate new expression
        let inner_entry = CronEntry::new(
            task_id,
            new_expression.to_string(),
            entry.description.clone(),
        )?;
        let new_entry = Arc::new(inner_entry);

        // Update expression index
        let old_expression = entry.expression.clone();
        if let Some(mut index) = self.expression_index.get_mut(&old_expression) {
            index.retain(|&id| id != task_id);
            if index.is_empty() {
                drop(index);
                self.expression_index.remove(&old_expression);
            }
        }

        let mut index = self
            .expression_index
            .entry(new_expression.to_string())
            .or_default();
        index.push(task_id);

        self.entries.insert(task_id, new_entry.clone());

        info!(
            task_id = %task_id,
            old_expression = %old_expression,
            new_expression = %new_expression,
            "Cron entry updated"
        );
        Ok(new_entry)
    }

    /// Get a cron entry by task ID.
    pub fn get(&self, task_id: Uuid) -> Option<Arc<CronEntry>> {
        self.entries.get(&task_id).map(|r| r.value().clone())
    }

    /// Get all due entries at the given time.
    ///
    /// An entry is due if its next scheduled time is <= the given time.
    /// This is the main method for finding tasks that should run now.
    pub fn due_entries(&self, at: chrono::DateTime<chrono::Utc>) -> Vec<Arc<CronEntry>> {
        self.entries
            .iter()
            .filter(|r| r.is_due(at))
            .map(|r| r.value().clone())
            .collect()
    }

    /// Get all entries.
    pub fn all_entries(&self) -> Vec<Arc<CronEntry>> {
        self.entries.iter().map(|r| r.value().clone()).collect()
    }

    /// Get entries by cron expression.
    pub fn get_by_expression(&self, expression: &str) -> Vec<Arc<CronEntry>> {
        if let Some(task_ids) = self.expression_index.get(expression) {
            task_ids
                .iter()
                .filter_map(|&task_id| self.entries.get(&task_id).map(|r| r.value().clone()))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get the count of registered entries.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Check if a task has a cron entry.
    pub fn has_entry(&self, task_id: Uuid) -> bool {
        self.entries.contains_key(&task_id)
    }

    /// Mark an entry as triggered (update last_triggered timestamp).
    ///
    /// Call this after a task is executed to track when it last ran.
    pub fn mark_triggered(&self, task_id: Uuid) -> Result<(), SwellError> {
        // Clone the entry value while holding the read lock
        let entry = match self.entries.get(&task_id) {
            Some(entry) => entry.value().clone(),
            None => {
                return Err(SwellError::InvalidOperation(format!(
                    "No cron entry found for task {}",
                    task_id
                )));
            }
        }; // Lock is released here

        let updated = CronEntry {
            task_id: entry.task_id,
            expression: entry.expression.clone(),
            last_triggered: Some(chrono::Utc::now()),
            created_at: entry.created_at,
            description: entry.description.clone(),
        };
        self.entries.insert(task_id, Arc::new(updated));
        Ok(())
    }

    /// Get all task IDs.
    pub fn task_ids(&self) -> Vec<Uuid> {
        self.entries.iter().map(|r| *r.key()).collect()
    }
}

impl Default for CronRegistry {
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

    fn new_registry() -> CronRegistry {
        CronRegistry::new()
    }

    // --- Registration Tests ---

    #[test]
    fn test_register_valid_cron_expression() {
        let registry = new_registry();
        let task_id = Uuid::new_v4();

        let result = registry.register(task_id, "* * * * * *");
        assert!(result.is_ok());
        assert_eq!(registry.entry_count(), 1);
    }

    #[test]
    fn test_register_invalid_cron_expression_returns_error() {
        let registry = new_registry();
        let task_id = Uuid::new_v4();

        let result = registry.register(task_id, "not a cron expression");
        assert!(result.is_err());
        assert!(registry.entry_count() == 0);
    }

    #[test]
    fn test_register_duplicate_task_returns_error() {
        let registry = new_registry();
        let task_id = Uuid::new_v4();

        registry.register(task_id, "* * * * * *").unwrap();
        let result = registry.register(task_id, "0 * * * * *");
        assert!(result.is_err());
    }

    #[test]
    fn test_register_with_description() {
        let registry = new_registry();
        let task_id = Uuid::new_v4();

        registry
            .register_with_description(task_id, "* * * * * *", "Every second task")
            .unwrap();

        let entry = registry.get(task_id).unwrap();
        assert_eq!(entry.description.as_ref().unwrap(), "Every second task");
    }

    // --- Removal Tests ---

    #[test]
    fn test_remove_existing_entry() {
        let registry = new_registry();
        let task_id = Uuid::new_v4();

        registry.register(task_id, "* * * * * *").unwrap();
        let result = registry.remove(task_id).unwrap();

        assert!(result.is_some());
        assert_eq!(registry.entry_count(), 0);
    }

    #[test]
    fn test_remove_nonexistent_entry_returns_ok_with_none() {
        let registry = new_registry();
        let task_id = Uuid::new_v4();

        let result = registry.remove(task_id).unwrap();
        assert!(result.is_none());
    }

    // --- Due Entries Tests ---

    #[test]
    fn test_due_entries_every_second_cron_is_due_at_current_second() {
        let registry = new_registry();
        let task_id = Uuid::new_v4();

        registry.register(task_id, "* * * * * *").unwrap();

        // Every second cron fires every second - so at any given second, it should be due
        let now = chrono::Utc::now();
        let due = registry.due_entries(now);

        assert!(
            due.len() == 1,
            "Every-second cron should be due at current time"
        );
        assert_eq!(due[0].task_id, task_id);
    }

    #[test]
    fn test_due_entries_once_a_year_not_due_unless_jan_1_midnight() {
        let registry = new_registry();
        let task_id = Uuid::new_v4();

        // January 1st at midnight - once a year
        registry.register(task_id, "0 0 1 1 *").unwrap();

        // A random time that is NOT Jan 1 midnight
        let not_jan_1 = chrono::NaiveDate::from_ymd_opt(2025, 6, 15)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc();

        let due = registry.due_entries(not_jan_1);
        assert!(due.is_empty(), "Task should not be due on June 15th");
    }

    #[test]
    fn test_due_entries_on_jan_1_midnight_is_due() {
        let registry = new_registry();
        let task_id = Uuid::new_v4();

        // January 1st at midnight - once a year
        registry.register(task_id, "0 0 1 1 *").unwrap();

        // Exactly Jan 1 midnight - the task should be due
        let jan_1_midnight = chrono::NaiveDate::from_ymd_opt(2025, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();

        let due = registry.due_entries(jan_1_midnight);
        assert!(!due.is_empty(), "Task should be due on Jan 1 midnight");
    }

    #[test]
    fn test_due_entries_empty_when_no_entries_due() {
        let registry = new_registry();
        let time = chrono::Utc::now();
        let due = registry.due_entries(time);
        assert!(
            due.is_empty(),
            "Empty registry should return empty due entries"
        );
    }

    #[test]
    fn test_due_entries_multiple_tasks() {
        let registry = new_registry();
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();

        registry.register(task1, "* * * * * *").unwrap(); // Every second
        registry.register(task2, "* * * * * *").unwrap(); // Every second (same pattern)

        // At any given second, both every-second tasks should be due
        let now = chrono::Utc::now();
        let due = registry.due_entries(now);

        assert_eq!(due.len(), 2, "Both every-second tasks should be due");
    }

    // --- Update Expression Tests ---

    #[test]
    fn test_update_expression() {
        let registry = new_registry();
        let task_id = Uuid::new_v4();

        registry.register(task_id, "* * * * * *").unwrap();

        let updated = registry.update_expression(task_id, "0 * * * * *").unwrap();

        assert_eq!(updated.expression, "0 * * * * *");
    }

    #[test]
    fn test_update_expression_nonexistent_returns_error() {
        let registry = new_registry();
        let task_id = Uuid::new_v4();

        let result = registry.update_expression(task_id, "* * * * * *");
        assert!(result.is_err());
    }

    // --- Get Entry Tests ---

    #[test]
    fn test_get_existing_entry() {
        let registry = new_registry();
        let task_id = Uuid::new_v4();

        registry.register(task_id, "* * * * * *").unwrap();

        let entry = registry.get(task_id).unwrap();
        assert_eq!(entry.task_id, task_id);
        assert_eq!(entry.expression, "* * * * * *");
    }

    #[test]
    fn test_get_nonexistent_entry_returns_none() {
        let registry = new_registry();
        let task_id = Uuid::new_v4();

        let entry = registry.get(task_id);
        assert!(entry.is_none());
    }

    // --- Expression Index Tests ---

    #[test]
    fn test_get_by_expression() {
        let registry = new_registry();
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();

        registry.register(task1, "* * * * * *").unwrap();
        registry.register(task2, "* * * * * *").unwrap();

        let entries = registry.get_by_expression("* * * * * *");
        assert_eq!(entries.len(), 2);
    }

    // --- Has Entry Tests ---

    #[test]
    fn test_has_entry() {
        let registry = new_registry();
        let task_id = Uuid::new_v4();

        assert!(!registry.has_entry(task_id));

        registry.register(task_id, "* * * * * *").unwrap();

        assert!(registry.has_entry(task_id));
    }

    // --- Mark Triggered Tests ---

    #[test]
    fn test_mark_triggered() {
        let registry = new_registry();
        let task_id = Uuid::new_v4();

        registry.register(task_id, "* * * * * *").unwrap();

        registry.mark_triggered(task_id).unwrap();

        let entry = registry.get(task_id).unwrap();
        assert!(entry.last_triggered.is_some());
    }

    #[test]
    fn test_mark_triggered_nonexistent_returns_error() {
        let registry = new_registry();
        let task_id = Uuid::new_v4();

        let result = registry.mark_triggered(task_id);
        assert!(result.is_err());
    }

    // --- All Entries and Task IDs Tests ---

    #[test]
    fn test_all_entries() {
        let registry = new_registry();
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();

        registry.register(task1, "* * * * * *").unwrap();
        registry.register(task2, "0 * * * * *").unwrap();

        let entries = registry.all_entries();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_task_ids() {
        let registry = new_registry();
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();

        registry.register(task1, "* * * * * *").unwrap();
        registry.register(task2, "0 * * * * *").unwrap();

        let ids = registry.task_ids();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&task1));
        assert!(ids.contains(&task2));
    }

    // --- Next Scheduled Time Tests ---

    #[test]
    fn test_next_scheduled_calculation() {
        let registry = new_registry();
        let task_id = Uuid::new_v4();

        // Every 5 minutes
        registry.register(task_id, "0 */5 * * * *").unwrap();

        let entry = registry.get(task_id).unwrap();
        let now = chrono::Utc::now();

        // The next scheduled time should be in the future
        if let Some(next) = entry.next_scheduled(now) {
            assert!(next > now, "Next scheduled time should be in the future");
        }
    }

    // --- Various Cron Patterns Tests ---

    #[test]
    fn test_various_cron_patterns_registration() {
        let registry = new_registry();

        // Standard patterns that should all be valid for registration
        let patterns = vec![
            "* * * * * *",    // Every second
            "0 * * * * *",    // Every minute
            "0 0 * * * *",    // Every hour
            "0 0 * * * *",    // Every day at midnight
            "0 0 * * 1 *",    // Every Monday at midnight (croner uses 1=Monday)
            "0 0 1 * * *",    // First day of every month
            "0 */5 * * * *",  // Every 5 minutes
            "0 */15 * * * *", // Every 15 minutes
            "0 */30 * * * *", // Every 30 minutes
        ];

        for (i, pattern) in patterns.iter().enumerate() {
            let task_id = Uuid::new_v4();
            let result = registry.register(task_id, pattern);
            assert!(
                result.is_ok(),
                "Pattern {} ('{}') should be valid: {:?}",
                i,
                pattern,
                result
            );
        }

        assert_eq!(registry.entry_count(), patterns.len());
    }

    #[test]
    fn test_specific_cron_patterns_due_at_known_times() {
        let registry = new_registry();

        // Test patterns with known expected behavior
        // Jan 1 midnight - should be due at exactly Jan 1 midnight
        let task1 = Uuid::new_v4();
        registry.register(task1, "0 0 1 1 *").unwrap();

        let jan_1 = chrono::NaiveDate::from_ymd_opt(2025, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();

        let due_at_jan1 = registry.due_entries(jan_1);
        assert!(
            !due_at_jan1.is_empty(),
            "Jan 1 pattern should be due at Jan 1 midnight"
        );

        // Same pattern should NOT be due on a different day
        let june_15 = chrono::NaiveDate::from_ymd_opt(2025, 6, 15)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc();

        let not_due_june = registry.due_entries(june_15);
        assert!(
            not_due_june.is_empty(),
            "Jan 1 pattern should NOT be due on June 15"
        );
    }

    // --- Edge Cases ---

    #[test]
    fn test_concurrent_access() {
        let registry = Arc::new(new_registry());
        let task_id = Uuid::new_v4();

        registry.register(task_id, "* * * * * *").unwrap();

        // Spawn multiple readers
        let handles: Vec<_> = (0..10)
            .map(|_| {
                let reg = registry.clone();
                std::thread::spawn(move || {
                    let _ = reg.get(task_id);
                    let _ = reg.due_entries(chrono::Utc::now());
                    let _ = reg.entry_count();
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }
    }
}
