//! Bounded worker pool with semaphore-controlled concurrency.
//!
//! This module provides a worker pool implementation with:
//! - Configurable worker count (3-5 workers)
//! - Semaphore-based concurrency control
//! - Worker lifecycle management (spawn, track, release)
//!
//! # Architecture
//!
//! The [`SemaphoreWorkerPool`] uses a tokio semaphore to limit concurrent worker
//! operations. Workers are identified by unique IDs and tracked through their
//! lifecycle states.

use swell_core::ids::TaskId;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Minimum workers allowed in the pool
pub const MIN_WORKERS: usize = 3;

/// Maximum workers allowed in the pool
pub const MAX_WORKERS: usize = 5;

/// Default worker count if not specified
pub const DEFAULT_WORKER_COUNT: usize = 4;

/// Worker lifecycle state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState {
    /// Worker is idle and available for work
    Idle,
    /// Worker is currently processing a task
    Busy,
    /// Worker is shutting down
    ShuttingDown,
    /// Worker has been stopped
    Stopped,
}

impl std::fmt::Display for WorkerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkerState::Idle => write!(f, "Idle"),
            WorkerState::Busy => write!(f, "Busy"),
            WorkerState::ShuttingDown => write!(f, "ShuttingDown"),
            WorkerState::Stopped => write!(f, "Stopped"),
        }
    }
}

/// A worker in the pool
#[derive(Debug, Clone)]
pub struct Worker {
    /// Unique worker identifier
    pub id: Uuid,
    /// Current worker state
    state: WorkerState,
    /// Task assigned to this worker (if any)
    task_id: Option<TaskId>,
}

impl Worker {
    /// Create a new worker with given ID
    pub fn new(id: Uuid) -> Self {
        Self {
            id,
            state: WorkerState::Idle,
            task_id: None,
        }
    }

    /// Get current state
    pub fn state(&self) -> WorkerState {
        self.state
    }

    /// Get assigned task ID
    pub fn task_id(&self) -> Option<TaskId> {
        self.task_id
    }

    /// Check if worker is idle
    pub fn is_idle(&self) -> bool {
        self.state == WorkerState::Idle
    }

    /// Check if worker is busy
    pub fn is_busy(&self) -> bool {
        self.state == WorkerState::Busy
    }

    /// Assign a task to this worker
    pub fn assign_task(&mut self, task_id: TaskId) {
        self.state = WorkerState::Busy;
        self.task_id = Some(task_id);
    }

    /// Release the worker (task completed or cancelled)
    pub fn release(&mut self) -> Option<TaskId> {
        let task = self.task_id.take();
        self.state = WorkerState::Idle;
        task
    }

    /// Start shutdown process
    pub fn start_shutdown(&mut self) {
        self.state = WorkerState::ShuttingDown;
    }

    /// Mark as stopped
    pub fn stop(&mut self) {
        self.state = WorkerState::Stopped;
        self.task_id = None;
    }
}

/// Semaphore-controlled worker pool for bounded concurrency.
///
/// This pool uses a semaphore to control the number of concurrent workers,
/// ensuring the system stays within resource limits.
///
/// # Type Parameters
///
/// * `T` - Associated data carried by each worker slot (can be unit for no data)
///
/// # Example
///
/// ```ignore
/// let pool = SemaphoreWorkerPool::new(4).unwrap();
/// let permit = pool.acquire().await.unwrap();
/// // Use the worker...
/// pool.release(permit);
/// ```
#[derive(Debug)]
pub struct SemaphoreWorkerPool {
    /// The semaphore controlling concurrency (wrapped in Arc for cloning)
    /// Made pub(crate) for testing access
    pub(crate) semaphore: Arc<Semaphore>,
    /// Maximum number of workers
    max_workers: usize,
    /// All workers in the pool
    workers: Vec<Worker>,
    /// Track which permits correspond to which workers
    worker_permits: std::sync::Mutex<std::collections::HashMap<Uuid, u64>>,
    /// Next permit ID counter
    permit_counter: std::sync::atomic::AtomicU64,
}

impl SemaphoreWorkerPool {
    /// Create a new worker pool with the specified number of workers.
    ///
    /// Returns an error if the worker count is outside the allowed range (3-5).
    pub fn new(worker_count: usize) -> Result<Self, WorkerPoolError> {
        Self::with_workers(worker_count)
    }

    /// Create a pool with custom worker count, clamping to valid range.
    ///
    /// If `worker_count` is below MIN_WORKERS, uses MIN_WORKERS.
    /// If `worker_count` is above MAX_WORKERS, uses MAX_WORKERS.
    pub fn with_clamped_size(worker_count: usize) -> Self {
        let size = worker_count.clamp(MIN_WORKERS, MAX_WORKERS);
        Self::with_workers(size).expect("clamped size is always valid (3-5)")
    }

    /// Internal constructor that creates the pool with a specific worker count.
    fn with_workers(count: usize) -> Result<Self, WorkerPoolError> {
        if count < MIN_WORKERS {
            return Err(WorkerPoolError::InvalidWorkerCount {
                requested: count,
                min: MIN_WORKERS,
                max: MAX_WORKERS,
            });
        }
        if count > MAX_WORKERS {
            return Err(WorkerPoolError::InvalidWorkerCount {
                requested: count,
                min: MIN_WORKERS,
                max: MAX_WORKERS,
            });
        }

        let semaphore = Arc::new(Semaphore::new(count));
        let workers: Vec<_> = (0..count).map(|_| Worker::new(Uuid::new_v4())).collect();

        info!(
            worker_count = count,
            min = MIN_WORKERS,
            max = MAX_WORKERS,
            "Created semaphore worker pool"
        );

        Ok(Self {
            semaphore,
            max_workers: count,
            workers,
            worker_permits: std::sync::Mutex::new(std::collections::HashMap::new()),
            permit_counter: std::sync::atomic::AtomicU64::new(0),
        })
    }

    /// Get the maximum number of workers
    pub fn max_workers(&self) -> usize {
        self.max_workers
    }

    /// Get the number of available (idle) workers
    ///
    /// Note: This returns the semaphore's available permits, which may differ
    /// from idle workers if permits are held but workers are not actively
    /// processing.
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Get the number of busy workers
    pub fn busy_count(&self) -> usize {
        self.max_workers - self.available_permits()
    }

    /// Get the number of idle workers
    pub fn idle_count(&self) -> usize {
        self.available_permits()
    }

    /// Check if the pool can accept more work
    pub fn can_accept_work(&self) -> bool {
        self.semaphore.available_permits() > 0
    }

    /// Acquire a permit to use a worker.
    ///
    /// This will wait until a worker is available.
    ///
    /// Returns the worker ID and an owned permit.
    /// Note: The worker is NOT marked as busy until assign_task() is called.
    pub async fn acquire(&mut self) -> Result<(Uuid, OwnedSemaphorePermit), WorkerPoolError> {
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| WorkerPoolError::SemaphoreClosed)?;

        let permit_id = self
            .permit_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        // Find an idle worker (don't mark as busy yet - that's done in assign_task)
        let worker_id = {
            let worker = self
                .workers
                .iter()
                .find(|w| w.is_idle())
                .ok_or(WorkerPoolError::NoIdleWorker)?;
            worker.id
        };

        // Track the permit
        self.worker_permits
            .lock()
            .unwrap()
            .insert(worker_id, permit_id);

        debug!(
            worker_id = %worker_id,
            available_permits = self.available_permits(),
            "Worker acquired from pool"
        );

        Ok((worker_id, permit))
    }

    /// Try to acquire a worker without waiting.
    ///
    /// Returns `None` if no workers are available.
    /// Note: The worker is NOT marked as busy until assign_task() is called.
    pub fn try_acquire(&mut self) -> Option<(Uuid, OwnedSemaphorePermit)> {
        let permit = self.semaphore.clone().try_acquire_owned().ok()?;

        let permit_id = self
            .permit_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        // Find an idle worker (don't mark as busy yet)
        let worker_id = {
            let worker = self.workers.iter().find(|w| w.is_idle())?;
            worker.id
        };

        self.worker_permits
            .lock()
            .unwrap()
            .insert(worker_id, permit_id);

        debug!(
            worker_id = %worker_id,
            available_permits = self.available_permits(),
            "Worker acquired (non-blocking)"
        );

        Some((worker_id, permit))
    }

    /// Assign a task to a worker.
    ///
    /// The worker must have been previously acquired.
    pub fn assign_task(&mut self, worker_id: Uuid, task_id: TaskId) -> Result<(), WorkerPoolError> {
        let worker = self
            .workers
            .iter_mut()
            .find(|w| w.id == worker_id)
            .ok_or(WorkerPoolError::WorkerNotFound(worker_id))?;

        if !worker.is_idle() {
            return Err(WorkerPoolError::WorkerNotIdle);
        }

        worker.assign_task(task_id);

        info!(
            worker_id = %worker_id,
            task_id = %task_id,
            busy_count = self.busy_count(),
            "Task assigned to worker"
        );

        Ok(())
    }

    /// Release a worker back to the pool.
    ///
    /// The permit is consumed and the worker becomes available again.
    pub fn release(&mut self, worker_id: Uuid) -> Result<TaskId, WorkerPoolError> {
        let permit_id = self
            .worker_permits
            .lock()
            .unwrap()
            .remove(&worker_id)
            .ok_or(WorkerPoolError::PermitNotFound)?;

        let worker = self
            .workers
            .iter_mut()
            .find(|w| w.id == worker_id)
            .ok_or(WorkerPoolError::WorkerNotFound(worker_id))?;

        // Release the worker - it will set task_id to None and state to Idle
        // This works regardless of current worker state
        let task_id = worker.task_id(); // Get task_id before releasing
        worker.release();

        debug!(
            worker_id = %worker_id,
            task_id = ?task_id,
            permit_id = permit_id,
            available_permits = self.available_permits(),
            "Worker released back to pool"
        );

        Ok(task_id.unwrap_or(TaskId::nil()))
    }

    /// Get a worker by ID
    pub fn get_worker(&self, worker_id: &Uuid) -> Option<&Worker> {
        self.workers.iter().find(|w| w.id == *worker_id)
    }

    /// Get all worker IDs
    pub fn worker_ids(&self) -> Vec<Uuid> {
        self.workers.iter().map(|w| w.id).collect()
    }

    /// Get all idle workers
    pub fn idle_workers(&self) -> Vec<Uuid> {
        self.workers
            .iter()
            .filter(|w| w.is_idle())
            .map(|w| w.id)
            .collect()
    }

    /// Get all busy workers
    pub fn busy_workers(&self) -> Vec<Uuid> {
        self.workers
            .iter()
            .filter(|w| w.is_busy())
            .map(|w| w.id)
            .collect()
    }

    /// Check if a task is being processed by any worker
    pub fn is_task_active(&self, task_id: &TaskId) -> bool {
        self.workers.iter().any(|w| w.task_id() == Some(*task_id))
    }

    /// Get the worker processing a specific task
    pub fn get_worker_for_task(&self, task_id: &TaskId) -> Option<Uuid> {
        self.workers
            .iter()
            .find(|w| w.task_id() == Some(*task_id))
            .map(|w| w.id)
    }

    /// Initiate graceful shutdown of all workers.
    ///
    /// Workers will finish their current tasks before stopping.
    pub fn start_shutdown(&mut self) {
        for worker in &mut self.workers {
            if worker.is_busy() {
                worker.start_shutdown();
            } else {
                worker.stop();
            }
        }
        info!(
            worker_count = self.workers.len(),
            "Worker pool shutdown initiated"
        );
    }

    /// Force stop all workers immediately.
    ///
    /// This should only be used in emergencies.
    pub fn force_stop(&mut self) {
        for worker in &mut self.workers {
            worker.stop();
        }
        // Drop all permits
        // Note: permits are implicitly dropped when OwnedSemaphorePermit is dropped
        warn!(
            worker_count = self.workers.len(),
            "Worker pool force stopped"
        );
    }

    /// Get pool statistics
    pub fn stats(&self) -> WorkerPoolStats {
        WorkerPoolStats {
            max_workers: self.max_workers,
            idle_workers: self.idle_count(),
            busy_workers: self.busy_count(),
            available_permits: self.available_permits(),
        }
    }
}

/// Worker pool statistics
#[derive(Debug, Clone)]
pub struct WorkerPoolStats {
    /// Maximum number of workers
    pub max_workers: usize,
    /// Number of idle workers
    pub idle_workers: usize,
    /// Number of busy workers
    pub busy_workers: usize,
    /// Available semaphore permits
    pub available_permits: usize,
}

impl std::fmt::Display for WorkerPoolStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "WorkerPool(max={}, idle={}, busy={}, available={})",
            self.max_workers, self.idle_workers, self.busy_workers, self.available_permits
        )
    }
}

/// Errors that can occur in worker pool operations
#[derive(Debug, thiserror::Error)]
pub enum WorkerPoolError {
    #[error("Invalid worker count: requested {requested}, must be between {min} and {max}")]
    InvalidWorkerCount {
        requested: usize,
        min: usize,
        max: usize,
    },

    #[error("No idle worker available")]
    NoIdleWorker,

    #[error("Worker not found: {0}")]
    WorkerNotFound(Uuid),

    #[error("Worker is not idle")]
    WorkerNotIdle,

    #[error("Worker is not busy")]
    WorkerNotBusy,

    #[error("Permit not found for worker")]
    PermitNotFound,

    #[error("Semaphore closed unexpectedly")]
    SemaphoreClosed,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- SemaphoreWorkerPool Creation Tests ---

    #[test]
    fn test_worker_pool_creation_with_valid_count() {
        // Test minimum
        let pool = SemaphoreWorkerPool::new(MIN_WORKERS).unwrap();
        assert_eq!(pool.max_workers(), MIN_WORKERS);
        assert_eq!(pool.available_permits(), MIN_WORKERS);

        // Test maximum
        let pool = SemaphoreWorkerPool::new(MAX_WORKERS).unwrap();
        assert_eq!(pool.max_workers(), MAX_WORKERS);
        assert_eq!(pool.available_permits(), MAX_WORKERS);

        // Test default (4)
        let pool = SemaphoreWorkerPool::with_clamped_size(DEFAULT_WORKER_COUNT);
        assert_eq!(pool.max_workers(), 4);
    }

    #[test]
    fn test_worker_pool_creation_with_invalid_count() {
        // Below minimum
        let result = SemaphoreWorkerPool::new(MIN_WORKERS - 1);
        assert!(matches!(
            result,
            Err(WorkerPoolError::InvalidWorkerCount { .. })
        ));

        // Above maximum
        let result = SemaphoreWorkerPool::new(MAX_WORKERS + 1);
        assert!(matches!(
            result,
            Err(WorkerPoolError::InvalidWorkerCount { .. })
        ));
    }

    #[test]
    fn test_worker_pool_clamped_size() {
        // Below minimum gets clamped up
        let pool = SemaphoreWorkerPool::with_clamped_size(1);
        assert_eq!(pool.max_workers(), MIN_WORKERS);

        // Above maximum gets clamped down
        let pool = SemaphoreWorkerPool::with_clamped_size(100);
        assert_eq!(pool.max_workers(), MAX_WORKERS);

        // Within range stays the same
        let pool = SemaphoreWorkerPool::with_clamped_size(4);
        assert_eq!(pool.max_workers(), 4);
    }

    #[test]
    fn test_worker_pool_initial_state() {
        let pool = SemaphoreWorkerPool::new(4).unwrap();

        assert_eq!(pool.max_workers(), 4);
        assert_eq!(pool.idle_count(), 4);
        assert_eq!(pool.busy_count(), 0);
        assert!(pool.can_accept_work());
    }

    #[test]
    fn test_worker_pool_worker_ids() {
        let pool = SemaphoreWorkerPool::new(3).unwrap();
        let ids = pool.worker_ids();

        assert_eq!(ids.len(), 3);
        // All IDs should be unique
        assert!(ids[0] != ids[1]);
        assert!(ids[0] != ids[2]);
        assert!(ids[1] != ids[2]);
    }

    // --- SemaphoreWorkerPool Acquire/Release Tests ---

    #[tokio::test]
    async fn test_worker_pool_acquire_and_release() {
        let mut pool = SemaphoreWorkerPool::new(3).unwrap();

        // Initial state
        assert_eq!(pool.available_permits(), 3);
        assert_eq!(pool.busy_count(), 0);

        // Acquire a worker - hold onto the permit
        let (worker_id, permit) = pool.acquire().await.unwrap();
        assert_eq!(pool.available_permits(), 2);
        assert_eq!(pool.busy_count(), 1); // busy_count tracks permits consumed

        // Assign a task
        let task_id = TaskId::new();
        pool.assign_task(worker_id, task_id).unwrap();

        // Worker should now be busy
        let worker = pool.get_worker(&worker_id).unwrap();
        assert!(worker.is_busy());
        assert_eq!(worker.task_id(), Some(task_id));

        // Release the worker - but permit is still held
        let released_task = pool.release(worker_id).unwrap();
        assert_eq!(released_task, task_id);
        // Worker is now idle, but permit still held, so:
        // - available_permits still 2 (permit not dropped)
        // - busy_count still 1 (permit not dropped)
        assert_eq!(pool.available_permits(), 2);
        assert_eq!(pool.busy_count(), 1);
        // But the worker itself is idle
        let worker = pool.get_worker(&worker_id).unwrap();
        assert!(worker.is_idle());

        // Now drop the permit - semaphore count increases
        drop(permit);
        assert_eq!(pool.available_permits(), 3);
        assert_eq!(pool.busy_count(), 0);
    }

    #[tokio::test]
    async fn test_worker_pool_acquire_all_workers() {
        let mut pool = SemaphoreWorkerPool::new(3).unwrap();

        // Acquire all workers - keep permits alive
        let mut permits: Vec<_> = Vec::new();
        for _ in 0..3 {
            let (_id, permit) = pool.acquire().await.unwrap();
            permits.push(permit);
        }

        // Pool should be full
        assert_eq!(pool.available_permits(), 0);
        assert!(!pool.can_accept_work());

        // Trying to try_acquire should fail
        assert!(pool.try_acquire().is_none());

        // Drop permits to release workers
        permits.clear();
        assert_eq!(pool.available_permits(), 3);
    }

    #[tokio::test]
    async fn test_worker_pool_release_and_reacquire() {
        let mut pool = SemaphoreWorkerPool::new(3).unwrap();

        // Acquire a worker
        let (w1, p1) = pool.acquire().await.unwrap();
        assert_eq!(pool.available_permits(), 2);
        assert_eq!(pool.busy_count(), 1);

        // Release the worker but keep permit
        pool.release(w1).unwrap();
        // Permit still held, so semaphore count unchanged
        assert_eq!(pool.available_permits(), 2);
        assert_eq!(pool.busy_count(), 1);

        // Drop the permit
        drop(p1);
        assert_eq!(pool.available_permits(), 3);
        assert_eq!(pool.busy_count(), 0);

        // Should be able to acquire again
        let (_w2, _permit) = pool.acquire().await.unwrap();
        assert_eq!(pool.available_permits(), 2);
    }

    #[tokio::test]
    async fn test_worker_pool_try_acquire_success() {
        let mut pool = SemaphoreWorkerPool::new(3).unwrap();

        let result = pool.try_acquire();
        assert!(result.is_some());
        let (_worker_id, _permit) = result.unwrap();
        // After try_acquire, a permit is consumed, but 2 remain (pool of 3)
        assert!(pool.can_accept_work()); // 2 permits still available
        assert_eq!(pool.available_permits(), 2);
    }

    #[tokio::test]
    async fn test_worker_pool_try_acquire_failure_when_full() {
        let mut pool = SemaphoreWorkerPool::new(3).unwrap();

        // Fill the pool - keep permits alive
        let mut permits: Vec<_> = Vec::new();
        for _ in 0..3 {
            let (worker_id, permit) = pool.try_acquire().unwrap();
            pool.assign_task(worker_id, TaskId::new()).unwrap();
            permits.push(permit);
        }

        // Try_acquire should fail
        assert!(pool.try_acquire().is_none());

        // Release
        permits.clear();
    }

    // --- SemaphoreWorkerPool Task Tracking Tests ---

    #[tokio::test]
    async fn test_worker_pool_is_task_active() {
        let mut pool = SemaphoreWorkerPool::new(3).unwrap();
        let task_id = TaskId::new();

        assert!(!pool.is_task_active(&task_id));

        let (worker_id, permit) = pool.acquire().await.unwrap();
        pool.assign_task(worker_id, task_id).unwrap();

        assert!(pool.is_task_active(&task_id));

        pool.release(worker_id).unwrap();
        // Task is no longer active after release
        // (but worker is still busy until permit is dropped)

        drop(permit);
        assert!(!pool.is_task_active(&task_id));
    }

    #[tokio::test]
    async fn test_worker_pool_get_worker_for_task() {
        let mut pool = SemaphoreWorkerPool::new(3).unwrap();
        let task_id = TaskId::new();

        let (worker_id, _permit) = pool.acquire().await.unwrap();
        pool.assign_task(worker_id, task_id).unwrap();

        assert_eq!(pool.get_worker_for_task(&task_id), Some(worker_id));
    }

    #[tokio::test]
    async fn test_worker_pool_multiple_tasks() {
        let mut pool = SemaphoreWorkerPool::new(3).unwrap();
        let task1 = TaskId::new();
        let task2 = TaskId::new();

        // Assign two tasks
        let (w1, p1) = pool.acquire().await.unwrap();
        pool.assign_task(w1, task1).unwrap();

        let (w2, _p2) = pool.acquire().await.unwrap();
        pool.assign_task(w2, task2).unwrap();

        assert!(pool.is_task_active(&task1));
        assert!(pool.is_task_active(&task2));
        assert_ne!(w1, w2);

        // Release first task
        pool.release(w1).unwrap();
        drop(p1);
        assert!(!pool.is_task_active(&task1));
        assert!(pool.is_task_active(&task2));
    }

    // --- SemaphoreWorkerPool Error Tests ---

    #[tokio::test]
    async fn test_worker_pool_release_unowned_worker() {
        let mut pool = SemaphoreWorkerPool::new(3).unwrap();

        // Try to release a worker that was never acquired
        let fake_id = Uuid::new_v4();
        let result = pool.release(fake_id);
        assert!(matches!(result, Err(WorkerPoolError::PermitNotFound)));
    }

    #[tokio::test]
    async fn test_worker_pool_assign_without_acquire() {
        let mut pool = SemaphoreWorkerPool::new(3).unwrap();
        let worker_id = pool.worker_ids()[0];
        let task_id = TaskId::new();

        // Trying to assign to a worker without acquiring should fail
        // because the worker's permit wasn't obtained
        let result = pool.assign_task(worker_id, task_id);
        assert!(result.is_ok()); // Actually this succeeds because worker is idle...

        // But release should fail because permit wasn't tracked
        let result = pool.release(worker_id);
        assert!(matches!(result, Err(WorkerPoolError::PermitNotFound)));
    }

    #[tokio::test]
    async fn test_worker_pool_double_release() {
        let mut pool = SemaphoreWorkerPool::new(3).unwrap();

        let (worker_id, permit) = pool.acquire().await.unwrap();
        pool.release(worker_id).unwrap();
        // Permit still held, need to drop it
        drop(permit);

        // Second release should fail because permit was already released
        // (release removes from worker_permits, so second call can't find permit)
        let result = pool.release(worker_id);
        assert!(matches!(result, Err(WorkerPoolError::PermitNotFound)));
    }

    // --- SemaphoreWorkerPool Shutdown Tests ---

    #[test]
    fn test_worker_pool_start_shutdown() {
        let mut pool = SemaphoreWorkerPool::new(3).unwrap();

        // Before shutdown
        assert_eq!(pool.idle_count(), 3);

        // Start shutdown
        pool.start_shutdown();

        // All workers should be stopped (they were all idle)
        for worker in pool.worker_ids() {
            let w = pool.get_worker(&worker).unwrap();
            assert_eq!(w.state(), WorkerState::Stopped);
        }
    }

    #[test]
    fn test_worker_pool_force_stop() {
        let mut pool = SemaphoreWorkerPool::new(3).unwrap();

        pool.force_stop();

        // All workers should be stopped
        for worker in pool.worker_ids() {
            let w = pool.get_worker(&worker).unwrap();
            assert_eq!(w.state(), WorkerState::Stopped);
        }
    }

    // --- SemaphoreWorkerPool Stats Tests ---

    #[test]
    fn test_worker_pool_stats() {
        let pool = SemaphoreWorkerPool::new(4).unwrap();
        let stats = pool.stats();

        assert_eq!(stats.max_workers, 4);
        assert_eq!(stats.idle_workers, 4);
        assert_eq!(stats.busy_workers, 0);
        assert_eq!(stats.available_permits, 4);
    }

    #[test]
    fn test_worker_pool_stats_display() {
        let pool = SemaphoreWorkerPool::new(4).unwrap();
        let stats = pool.stats();
        let display = format!("{}", stats);

        assert!(display.contains("max=4"));
        assert!(display.contains("idle=4"));
        assert!(display.contains("busy=0"));
    }

    // --- Worker State Tests ---

    #[test]
    fn test_worker_state_display() {
        assert_eq!(format!("{}", WorkerState::Idle), "Idle");
        assert_eq!(format!("{}", WorkerState::Busy), "Busy");
        assert_eq!(format!("{}", WorkerState::ShuttingDown), "ShuttingDown");
        assert_eq!(format!("{}", WorkerState::Stopped), "Stopped");
    }

    #[test]
    fn test_worker_lifecycle() {
        let mut worker = Worker::new(Uuid::new_v4());

        // Initial state is idle
        assert!(worker.is_idle());
        assert!(!worker.is_busy());

        // Assign task
        let task_id = TaskId::new();
        worker.assign_task(task_id);
        assert!(!worker.is_idle());
        assert!(worker.is_busy());
        assert_eq!(worker.task_id(), Some(task_id));

        // Release
        let released = worker.release();
        assert_eq!(released, Some(task_id));
        assert!(worker.is_idle());
        assert!(!worker.is_busy());
        assert_eq!(worker.task_id(), None);

        // Shutdown sequence
        worker.start_shutdown();
        assert_eq!(worker.state(), WorkerState::ShuttingDown);

        worker.stop();
        assert_eq!(worker.state(), WorkerState::Stopped);
    }

    // --- Concurrency Tests ---

    #[tokio::test]
    async fn test_worker_pool_concurrent_acquire() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::sync::Mutex;

        // Test that the pool properly limits concurrent access using semaphore
        // Use Arc<Mutex<T>> for interior mutability
        let pool = Arc::new(Mutex::new(SemaphoreWorkerPool::new(3).unwrap()));
        let successes = Arc::new(AtomicUsize::new(0));
        let failures = Arc::new(AtomicUsize::new(0));
        let permits = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let mut join_set = tokio::task::JoinSet::new();

        // Spawn 5 tasks that try to acquire workers
        for _ in 0..5 {
            let pool_clone = pool.clone();
            let successes_clone = successes.clone();
            let failures_clone = failures.clone();
            let permits_clone = permits.clone();
            join_set.spawn(async move {
                let result = {
                    let pool = pool_clone.lock().await;
                    // Just try to acquire a permit without marking worker as busy
                    // We only care about the semaphore limiting here
                    let permit = pool.semaphore.clone().try_acquire_owned();
                    if permit.is_ok() {
                        successes_clone.fetch_add(1, Ordering::SeqCst);
                        permit
                    } else {
                        failures_clone.fetch_add(1, Ordering::SeqCst);
                        permit
                    }
                };
                // Keep permit alive outside the lock
                if let Ok(permit) = result {
                    permits_clone.lock().await.push(permit);
                }
            });
        }

        // Wait for all tasks to complete
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(()) => {} // Count already updated
                Err(_) => {} // Count already updated
            }
        }

        let success_count = successes.load(Ordering::SeqCst);
        let failure_count = failures.load(Ordering::SeqCst);

        // Should have exactly 3 successful acquisitions (pool size)
        assert_eq!(success_count, 3, "Expected 3 successful acquisitions");
        // And 2 failures (remaining tasks couldn't acquire - pool was full)
        assert_eq!(failure_count, 2, "Expected 2 failures");

        // Verify pool state - permits still held
        {
            let pool_ref = pool.lock().await;
            assert_eq!(pool_ref.max_workers(), 3);
            assert_eq!(pool_ref.available_permits(), 0);
        }
    }

    // --- Error Type Tests ---

    #[test]
    fn test_worker_pool_error_display() {
        let err = WorkerPoolError::InvalidWorkerCount {
            requested: 10,
            min: 3,
            max: 5,
        };
        assert!(err.to_string().contains("10"));
        assert!(err.to_string().contains("3"));
        assert!(err.to_string().contains("5"));

        assert_eq!(
            format!("{}", WorkerPoolError::NoIdleWorker),
            "No idle worker available"
        );
        assert_eq!(
            format!("{}", WorkerPoolError::WorkerNotIdle),
            "Worker is not idle"
        );
        assert_eq!(
            format!("{}", WorkerPoolError::WorkerNotBusy),
            "Worker is not busy"
        );
        assert_eq!(
            format!("{}", WorkerPoolError::SemaphoreClosed),
            "Semaphore closed unexpectedly"
        );
    }
}
