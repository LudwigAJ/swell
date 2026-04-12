//! File lock manager for preventing concurrent edits to the same file.
//!
//! This module provides logical file locks to prevent multiple agents from
//! editing the same file simultaneously. It supports:
//! - Acquiring locks on file paths
//! - Conflict detection
//! - Lock release on completion
//!
//! # Architecture
//!
//! The [`FileLockManager`] uses a tokio RwLock for thread-safe access to the
//! lock state. Locks are identified by unique IDs and track which task holds them.

use std::collections::HashMap;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// A file lock representing exclusive access to a file path.
#[derive(Debug, Clone)]
pub struct FileLock {
    /// Unique lock identifier
    pub id: Uuid,
    /// File path being locked
    pub path: String,
    /// Task ID that owns this lock
    pub task_id: Uuid,
    /// Agent ID that acquired this lock
    pub agent_id: Option<Uuid>,
}

impl FileLock {
    /// Create a new file lock
    pub fn new(path: String, task_id: Uuid, agent_id: Option<Uuid>) -> Self {
        Self {
            id: Uuid::new_v4(),
            path,
            task_id,
            agent_id,
        }
    }

    /// Check if this lock is held by a specific task
    pub fn is_held_by(&self, task_id: Uuid) -> bool {
        self.task_id == task_id
    }

    /// Check if this lock is held by a specific agent
    pub fn is_held_by_agent(&self, agent_id: Uuid) -> bool {
        self.agent_id == Some(agent_id)
    }
}

/// Result of a lock acquisition attempt
#[derive(Debug)]
pub enum LockAcquisitionResult {
    /// Lock was successfully acquired
    Acquired(FileLock),
    /// Lock is held by another task (conflict)
    Conflict {
        existing_lock: FileLock,
        requested_by: Uuid,
    },
    /// Lock is already held by the same task (re-acquisition)
    AlreadyHeld { existing_lock: FileLock },
}

/// Manager for file locks.
///
/// Provides thread-safe operations for acquiring, releasing, and checking file locks
/// to prevent concurrent edits to the same file across agents.
#[derive(Debug)]
pub struct FileLockManager {
    /// Locks by file path
    locks: RwLock<HashMap<String, FileLock>>,
    /// Lock history for debugging
    lock_history: RwLock<Vec<LockEvent>>,
}

/// A lock-related event for audit trail
#[derive(Debug, Clone)]
pub struct LockEvent {
    /// Event type
    pub event_type: LockEventType,
    /// Lock ID
    pub lock_id: Uuid,
    /// File path
    pub path: String,
    /// Task ID
    pub task_id: Uuid,
    /// Agent ID (if applicable)
    pub agent_id: Option<Uuid>,
    /// Timestamp (Unix epoch millis)
    pub timestamp: i64,
}

/// Type of lock event
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockEventType {
    Acquired,
    Released,
    Conflict,
    Expired,
}

impl FileLockManager {
    /// Create a new file lock manager
    pub fn new() -> Self {
        Self {
            locks: RwLock::new(HashMap::new()),
            lock_history: RwLock::new(Vec::new()),
        }
    }

    /// Get the number of active locks
    pub async fn active_lock_count(&self) -> usize {
        self.locks.read().await.len()
    }

    /// Check if a file is locked
    pub async fn is_locked(&self, path: &str) -> bool {
        self.locks.read().await.contains_key(path)
    }

    /// Check if a file is locked by a specific task
    pub async fn is_locked_by(&self, path: &str, task_id: Uuid) -> bool {
        self.locks
            .read()
            .await
            .get(path)
            .map(|lock| lock.is_held_by(task_id))
            .unwrap_or(false)
    }

    /// Get the lock for a file path, if any
    pub async fn get_lock(&self, path: &str) -> Option<FileLock> {
        self.locks.read().await.get(path).cloned()
    }

    /// Get all active locks for a task
    pub async fn get_task_locks(&self, task_id: Uuid) -> Vec<FileLock> {
        self.locks
            .read()
            .await
            .values()
            .filter(|lock| lock.is_held_by(task_id))
            .cloned()
            .collect()
    }

    /// Get all active locks
    pub async fn get_all_locks(&self) -> Vec<FileLock> {
        self.locks.read().await.values().cloned().collect()
    }

    /// Acquire a lock on a file path for a task.
    ///
    /// Returns `LockAcquisitionResult::Acquired` if the lock was acquired successfully.
    /// Returns `LockAcquisitionResult::Conflict` if another task holds the lock.
    /// Returns `LockAcquisitionResult::AlreadyHeld` if the same task already holds the lock.
    pub async fn acquire(
        &self,
        path: String,
        task_id: Uuid,
        agent_id: Option<Uuid>,
    ) -> LockAcquisitionResult {
        let mut locks = self.locks.write().await;

        // Check if already locked
        if let Some(existing_lock) = locks.get(&path) {
            if existing_lock.is_held_by(task_id) {
                // Same task already holds the lock
                debug!(
                    path = %path,
                    task_id = %task_id,
                    lock_id = %existing_lock.id,
                    "Lock already held by task"
                );
                return LockAcquisitionResult::AlreadyHeld {
                    existing_lock: existing_lock.clone(),
                };
            } else {
                // Another task holds the lock - conflict
                warn!(
                    path = %path,
                    existing_task_id = %existing_lock.task_id,
                    requested_by = %task_id,
                    "Lock conflict detected"
                );
                self.record_event(
                    LockEventType::Conflict,
                    existing_lock.id,
                    path.clone(),
                    task_id,
                    agent_id,
                )
                .await;
                return LockAcquisitionResult::Conflict {
                    existing_lock: existing_lock.clone(),
                    requested_by: task_id,
                };
            }
        }

        // Create new lock
        let lock = FileLock::new(path.clone(), task_id, agent_id);
        locks.insert(path.clone(), lock.clone());

        info!(
            path = %path,
            task_id = %task_id,
            lock_id = %lock.id,
            "File lock acquired"
        );

        self.record_event(LockEventType::Acquired, lock.id, path, task_id, agent_id)
            .await;

        LockAcquisitionResult::Acquired(lock)
    }

    /// Release a lock on a file path.
    ///
    /// Returns the released lock if it existed and was released, or None if no lock was held.
    pub async fn release(&self, path: &str, task_id: Uuid) -> Option<FileLock> {
        let mut locks = self.locks.write().await;

        // Check if locked by this task
        if let Some(existing_lock) = locks.get(path) {
            if !existing_lock.is_held_by(task_id) {
                debug!(
                    path = %path,
                    holder_task_id = %existing_lock.task_id,
                    releasing_task_id = %task_id,
                    "Cannot release lock held by another task"
                );
                return None;
            }
        }

        // Remove the lock
        let removed = locks.remove(path)?;

        info!(
            path = %path,
            task_id = %task_id,
            lock_id = %removed.id,
            "File lock released"
        );

        self.record_event(
            LockEventType::Released,
            removed.id,
            path.to_string(),
            task_id,
            None,
        )
        .await;

        Some(removed)
    }

    /// Release all locks held by a task.
    ///
    /// Returns the number of locks released.
    pub async fn release_all_for_task(&self, task_id: Uuid) -> usize {
        let mut locks = self.locks.write().await;
        let mut released_count = 0;

        // Collect paths to remove (can't remove while iterating)
        let paths_to_release: Vec<String> = locks
            .values()
            .filter(|lock| lock.is_held_by(task_id))
            .map(|lock| lock.path.clone())
            .collect();

        for path in &paths_to_release {
            if let Some(removed) = locks.remove(path) {
                info!(
                    path = %path,
                    task_id = %task_id,
                    lock_id = %removed.id,
                    "File lock released (task cleanup)"
                );
                self.record_event(
                    LockEventType::Released,
                    removed.id,
                    path.clone(),
                    task_id,
                    None,
                )
                .await;
                released_count += 1;
            }
        }

        released_count
    }

    /// Force release a lock (admin operation).
    ///
    /// Use with caution - this removes the lock regardless of who holds it.
    pub async fn force_release(&self, path: &str) -> Option<FileLock> {
        let mut locks = self.locks.write().await;
        let removed = locks.remove(path)?;

        warn!(
            path = %path,
            task_id = %removed.task_id,
            lock_id = %removed.id,
            "File lock force released"
        );

        self.record_event(
            LockEventType::Expired,
            removed.id,
            path.to_string(),
            removed.task_id,
            None,
        )
        .await;

        Some(removed)
    }

    /// Check if acquiring a lock would conflict
    pub async fn would_conflict(&self, path: &str, task_id: Uuid) -> Option<FileLock> {
        let locks = self.locks.read().await;

        if let Some(existing_lock) = locks.get(path) {
            if !existing_lock.is_held_by(task_id) {
                return Some(existing_lock.clone());
            }
        }

        None
    }

    /// Get lock statistics
    pub async fn stats(&self) -> LockStats {
        let locks = self.locks.read().await;
        LockStats {
            active_locks: locks.len(),
            unique_tasks: locks
                .values()
                .map(|l| l.task_id)
                .collect::<std::collections::HashSet<_>>()
                .len(),
        }
    }

    /// Record a lock event in the history
    async fn record_event(
        &self,
        event_type: LockEventType,
        lock_id: Uuid,
        path: String,
        task_id: Uuid,
        agent_id: Option<Uuid>,
    ) {
        let event = LockEvent {
            event_type,
            lock_id,
            path,
            task_id,
            agent_id,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
        };
        self.lock_history.write().await.push(event);
    }

    /// Get lock history (for debugging)
    pub async fn get_history(&self) -> Vec<LockEvent> {
        self.lock_history.read().await.clone()
    }
}

impl Default for FileLockManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Lock statistics
#[derive(Debug, Clone)]
pub struct LockStats {
    /// Number of active locks
    pub active_locks: usize,
    /// Number of unique tasks with locks
    pub unique_tasks: usize,
}

/// Errors that can occur during file lock operations
#[derive(Debug, thiserror::Error)]
pub enum FileLockError {
    #[error("Lock conflict: file '{0}' is locked by another task")]
    Conflict(String),

    #[error("Lock not found for file '{0}'")]
    NotFound(String),

    #[error("Task {0} does not hold lock on file '{1}'")]
    NotHolder(Uuid, String),
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- FileLock Tests ---

    #[test]
    fn test_file_lock_creation() {
        let path = "/path/to/file.rs".to_string();
        let task_id = Uuid::new_v4();
        let agent_id = Some(Uuid::new_v4());

        let lock = FileLock::new(path.clone(), task_id, agent_id);

        assert_eq!(lock.path, path);
        assert_eq!(lock.task_id, task_id);
        assert_eq!(lock.agent_id, agent_id);
        assert_ne!(lock.id, Uuid::nil());
    }

    #[test]
    fn test_file_lock_is_held_by() {
        let task_id = Uuid::new_v4();
        let other_task_id = Uuid::new_v4();

        let lock = FileLock::new("test.rs".to_string(), task_id, None);

        assert!(lock.is_held_by(task_id));
        assert!(!lock.is_held_by(other_task_id));
    }

    #[test]
    fn test_file_lock_is_held_by_agent() {
        let agent_id = Uuid::new_v4();
        let other_agent_id = Uuid::new_v4();

        let lock = FileLock::new("test.rs".to_string(), Uuid::new_v4(), Some(agent_id));

        assert!(lock.is_held_by_agent(agent_id));
        assert!(!lock.is_held_by_agent(other_agent_id));
    }

    // --- FileLockManager Creation Tests ---

    #[tokio::test]
    async fn test_manager_initial_state() {
        let manager = FileLockManager::new();

        assert_eq!(manager.active_lock_count().await, 0);
        assert!(!manager.is_locked("test.rs").await);
        assert!(manager.get_all_locks().await.is_empty());
    }

    // --- FileLockManager Acquire Tests ---

    #[tokio::test]
    async fn test_acquire_lock_success() {
        let manager = FileLockManager::new();
        let task_id = Uuid::new_v4();
        let path = "/path/to/file.rs".to_string();

        let result = manager.acquire(path.clone(), task_id, None).await;

        match result {
            LockAcquisitionResult::Acquired(lock) => {
                assert_eq!(lock.path, path);
                assert_eq!(lock.task_id, task_id);
            }
            _ => panic!("Expected Acquired result"),
        }

        assert_eq!(manager.active_lock_count().await, 1);
        assert!(manager.is_locked(&path).await);
    }

    #[tokio::test]
    async fn test_acquire_lock_conflict() {
        let manager = FileLockManager::new();
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();
        let path = "/path/to/file.rs".to_string();

        // First task acquires lock
        let result1 = manager.acquire(path.clone(), task1, None).await;
        assert!(matches!(result1, LockAcquisitionResult::Acquired(_)));

        // Second task tries to acquire - should conflict
        let result2 = manager.acquire(path.clone(), task2, None).await;

        match result2 {
            LockAcquisitionResult::Conflict {
                existing_lock,
                requested_by,
            } => {
                assert_eq!(existing_lock.path, path);
                assert_eq!(existing_lock.task_id, task1);
                assert_eq!(requested_by, task2);
            }
            _ => panic!("Expected Conflict result"),
        }

        // Lock count should still be 1
        assert_eq!(manager.active_lock_count().await, 1);
    }

    #[tokio::test]
    async fn test_acquire_lock_already_held() {
        let manager = FileLockManager::new();
        let task_id = Uuid::new_v4();
        let path = "/path/to/file.rs".to_string();

        // First acquisition
        let result1 = manager.acquire(path.clone(), task_id, None).await;
        assert!(matches!(result1, LockAcquisitionResult::Acquired(_)));

        // Same task re-acquires - should return AlreadyHeld
        let result2 = manager.acquire(path.clone(), task_id, None).await;

        match result2 {
            LockAcquisitionResult::AlreadyHeld { existing_lock } => {
                assert_eq!(existing_lock.path, path);
                assert_eq!(existing_lock.task_id, task_id);
            }
            _ => panic!("Expected AlreadyHeld result"),
        }

        // Still only 1 lock
        assert_eq!(manager.active_lock_count().await, 1);
    }

    #[tokio::test]
    async fn test_acquire_multiple_different_files() {
        let manager = FileLockManager::new();
        let task_id = Uuid::new_v4();

        let result1 = manager.acquire("file1.rs".to_string(), task_id, None).await;
        let result2 = manager.acquire("file2.rs".to_string(), task_id, None).await;
        let result3 = manager.acquire("file3.rs".to_string(), task_id, None).await;

        assert!(matches!(result1, LockAcquisitionResult::Acquired(_)));
        assert!(matches!(result2, LockAcquisitionResult::Acquired(_)));
        assert!(matches!(result3, LockAcquisitionResult::Acquired(_)));

        assert_eq!(manager.active_lock_count().await, 3);
    }

    // --- FileLockManager Release Tests ---

    #[tokio::test]
    async fn test_release_lock_success() {
        let manager = FileLockManager::new();
        let task_id = Uuid::new_v4();
        let path = "/path/to/file.rs".to_string();

        manager.acquire(path.clone(), task_id, None).await;
        assert!(manager.is_locked(&path).await);

        let released = manager.release(&path, task_id).await;

        assert!(released.is_some());
        assert!(!manager.is_locked(&path).await);
        assert_eq!(manager.active_lock_count().await, 0);
    }

    #[tokio::test]
    async fn test_release_lock_not_found() {
        let manager = FileLockManager::new();
        let task_id = Uuid::new_v4();

        let released = manager.release("/nonexistent.rs", task_id).await;

        assert!(released.is_none());
    }

    #[tokio::test]
    async fn test_release_lock_not_holder() {
        let manager = FileLockManager::new();
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();
        let path = "/path/to/file.rs".to_string();

        manager.acquire(path.clone(), task1, None).await;

        // Task2 tries to release - should fail
        let released = manager.release(&path, task2).await;

        assert!(released.is_none());
        // Lock should still exist
        assert!(manager.is_locked(&path).await);
    }

    #[tokio::test]
    async fn test_release_all_for_task() {
        let manager = FileLockManager::new();
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();

        manager.acquire("file1.rs".to_string(), task1, None).await;
        manager.acquire("file2.rs".to_string(), task1, None).await;
        manager.acquire("file3.rs".to_string(), task2, None).await;

        let released_count = manager.release_all_for_task(task1).await;

        assert_eq!(released_count, 2);
        assert_eq!(manager.active_lock_count().await, 1);
        assert!(manager.is_locked("file3.rs").await);
    }

    #[tokio::test]
    async fn test_release_all_for_task_no_locks() {
        let manager = FileLockManager::new();
        let task_id = Uuid::new_v4();

        let released_count = manager.release_all_for_task(task_id).await;

        assert_eq!(released_count, 0);
    }

    // --- FileLockManager Query Tests ---

    #[tokio::test]
    async fn test_get_lock() {
        let manager = FileLockManager::new();
        let task_id = Uuid::new_v4();
        let path = "/path/to/file.rs".to_string();

        let result = manager.acquire(path.clone(), task_id, None).await;
        let acquired_lock = match result {
            LockAcquisitionResult::Acquired(lock) => lock,
            _ => panic!("Expected Acquired result"),
        };

        let retrieved = manager.get_lock(&path).await;

        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, acquired_lock.id);
    }

    #[tokio::test]
    async fn test_get_lock_not_found() {
        let manager = FileLockManager::new();

        let retrieved = manager.get_lock("/nonexistent.rs").await;

        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_get_task_locks() {
        let manager = FileLockManager::new();
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();

        manager.acquire("file1.rs".to_string(), task1, None).await;
        manager.acquire("file2.rs".to_string(), task1, None).await;
        manager.acquire("file3.rs".to_string(), task2, None).await;

        let task1_locks = manager.get_task_locks(task1).await;
        let task2_locks = manager.get_task_locks(task2).await;

        assert_eq!(task1_locks.len(), 2);
        assert_eq!(task2_locks.len(), 1);
    }

    #[tokio::test]
    async fn test_is_locked_by() {
        let manager = FileLockManager::new();
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();
        let path = "/path/to/file.rs".to_string();

        manager.acquire(path.clone(), task1, None).await;

        assert!(manager.is_locked_by(&path, task1).await);
        assert!(!manager.is_locked_by(&path, task2).await);
    }

    #[tokio::test]
    async fn test_would_conflict() {
        let manager = FileLockManager::new();
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();
        let path = "/path/to/file.rs".to_string();

        manager.acquire(path.clone(), task1, None).await;

        // Same task should not conflict
        assert!(manager.would_conflict(&path, task1).await.is_none());

        // Different task should conflict
        let conflict = manager.would_conflict(&path, task2).await;
        assert!(conflict.is_some());
        assert_eq!(conflict.unwrap().task_id, task1);
    }

    // --- FileLockManager Force Release Tests ---

    #[tokio::test]
    async fn test_force_release() {
        let manager = FileLockManager::new();
        let task_id = Uuid::new_v4();
        let path = "/path/to/file.rs".to_string();

        manager.acquire(path.clone(), task_id, None).await;
        assert!(manager.is_locked(&path).await);

        let released = manager.force_release(&path).await;

        assert!(released.is_some());
        assert!(!manager.is_locked(&path).await);
    }

    #[tokio::test]
    async fn test_force_release_not_found() {
        let manager = FileLockManager::new();

        let released = manager.force_release("/nonexistent.rs").await;

        assert!(released.is_none());
    }

    // --- FileLockManager Stats Tests ---

    #[tokio::test]
    async fn test_stats() {
        let manager = FileLockManager::new();
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();

        manager.acquire("file1.rs".to_string(), task1, None).await;
        manager.acquire("file2.rs".to_string(), task1, None).await;
        manager.acquire("file3.rs".to_string(), task2, None).await;

        let stats = manager.stats().await;

        assert_eq!(stats.active_locks, 3);
        assert_eq!(stats.unique_tasks, 2);
    }

    #[tokio::test]
    async fn test_stats_empty() {
        let manager = FileLockManager::new();

        let stats = manager.stats().await;

        assert_eq!(stats.active_locks, 0);
        assert_eq!(stats.unique_tasks, 0);
    }

    // --- FileLockManager History Tests ---

    #[tokio::test]
    async fn test_history_records_events() {
        let manager = FileLockManager::new();
        let task_id = Uuid::new_v4();
        let path = "/path/to/file.rs".to_string();

        manager.acquire(path.clone(), task_id, None).await;
        manager.release(&path, task_id).await;

        let history = manager.get_history().await;

        // Should have at least 2 events: Acquired and Released
        assert!(history.len() >= 2);

        let acquired_event = history
            .iter()
            .find(|e| e.event_type == LockEventType::Acquired);
        let released_event = history
            .iter()
            .find(|e| e.event_type == LockEventType::Released);

        assert!(acquired_event.is_some());
        assert!(released_event.is_some());
    }

    // --- Concurrent Access Tests ---

    #[tokio::test]
    async fn test_concurrent_acquire_same_file() {
        use std::sync::Arc;

        let manager = Arc::new(FileLockManager::new());
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();

        // Simulate concurrent acquisition attempts
        let manager1 = manager.clone();
        let manager2 = manager.clone();
        let path1 = "/path/to/file.rs".to_string();
        let path2 = "/path/to/file.rs".to_string();

        let handle1 = tokio::spawn(async move { manager1.acquire(path1, task1, None).await });

        let handle2 = tokio::spawn(async move {
            // Small delay to ensure handle1 acquires first
            tokio::time::sleep(tokio::time::Duration::from_micros(10)).await;
            manager2.acquire(path2, task2, None).await
        });

        let result1 = handle1.await.unwrap();
        let result2 = handle2.await.unwrap();

        // One should succeed, one should conflict
        let success_count = match (&result1, &result2) {
            (LockAcquisitionResult::Acquired(_), LockAcquisitionResult::Acquired(_)) => 2,
            (LockAcquisitionResult::Acquired(_), LockAcquisitionResult::Conflict { .. }) => 1,
            (LockAcquisitionResult::Conflict { .. }, LockAcquisitionResult::Acquired(_)) => 1,
            _ => 0,
        };

        assert_eq!(
            success_count, 1,
            "Expected exactly one successful acquisition"
        );
    }

    #[tokio::test]
    async fn test_concurrent_acquire_different_files() {
        use std::sync::Arc;

        let manager = Arc::new(FileLockManager::new());
        let task_id = Uuid::new_v4();

        // Acquire multiple different files concurrently
        let paths = vec!["file1.rs", "file2.rs", "file3.rs", "file4.rs", "file5.rs"];

        let mut handles = Vec::new();
        for path in paths {
            let manager_clone = manager.clone();
            let path_str = path.to_string();
            let task = task_id;
            handles.push(tokio::spawn(async move {
                manager_clone.acquire(path_str, task, None).await
            }));
        }

        // All should succeed
        let mut success_count = 0;
        for handle in handles {
            let result = handle.await.unwrap();
            if matches!(result, LockAcquisitionResult::Acquired(_)) {
                success_count += 1;
            }
        }

        assert_eq!(success_count, 5);
        assert_eq!(manager.active_lock_count().await, 5);
    }

    // --- LockEventType Tests ---

    #[test]
    fn test_lock_event_type_debug() {
        assert_eq!(format!("{:?}", LockEventType::Acquired), "Acquired");
        assert_eq!(format!("{:?}", LockEventType::Released), "Released");
        assert_eq!(format!("{:?}", LockEventType::Conflict), "Conflict");
        assert_eq!(format!("{:?}", LockEventType::Expired), "Expired");
    }

    // --- LockStats Tests ---

    #[test]
    fn test_lock_stats_debug() {
        let stats = LockStats {
            active_locks: 5,
            unique_tasks: 2,
        };
        let debug_str = format!("{:?}", stats);
        assert!(debug_str.contains("active_locks: 5"));
        assert!(debug_str.contains("unique_tasks: 2"));
    }

    // --- FileLockError Tests ---

    #[test]
    fn test_file_lock_error_display() {
        let err = FileLockError::Conflict("/path/file.rs".to_string());
        assert_eq!(
            err.to_string(),
            "Lock conflict: file '/path/file.rs' is locked by another task"
        );

        let err = FileLockError::NotFound("/path/file.rs".to_string());
        assert_eq!(err.to_string(), "Lock not found for file '/path/file.rs'");

        let task_id = Uuid::new_v4();
        let err = FileLockError::NotHolder(task_id, "/path/file.rs".to_string());
        assert!(err.to_string().contains("does not hold lock"));
    }
}
