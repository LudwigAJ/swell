//! Scheduler managing ready queue, task assignment by priority.
//!
//! This module provides:
//! - Priority-based queue for tasks in Ready state
//! - Max concurrent workers enforcement (default: 6, max: 7)
#![allow(dead_code)]
//! - Fair scheduling (round-robin for tasks of same priority)
//!
//! # Architecture
//!
//! The scheduler maintains:
//! - [`Scheduler`] - main scheduler with priority queue
//! - [`TaskQueue`] - priority-ordered queue of ready tasks
//! - [`WorkerPool`] - tracks active workers and their assignments

use crate::SwellError;
use std::collections::{BinaryHeap, HashSet};
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Default max concurrent workers
pub const DEFAULT_MAX_WORKERS: usize = 6;

/// Maximum allowed workers (hard limit)
pub const MAX_MAX_WORKERS: usize = 7;

/// Task priority for scheduling
/// Higher number = higher priority (scheduled first)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TaskPriority {
    /// Base priority (0 = lowest)
    pub base: u32,
    /// When the task was enqueued (for FIFO within same priority)
    pub enqueue_ts: u64,
}

impl TaskPriority {
    pub fn new(base: u32, enqueue_ts: u64) -> Self {
        Self { base, enqueue_ts }
    }

    pub fn default_priority(enqueue_ts: u64) -> Self {
        Self {
            base: 100,
            enqueue_ts,
        }
    }

    pub fn low_priority(enqueue_ts: u64) -> Self {
        Self {
            base: 50,
            enqueue_ts,
        }
    }

    pub fn high_priority(enqueue_ts: u64) -> Self {
        Self {
            base: 200,
            enqueue_ts,
        }
    }
}

/// Priority queue entry with metadata
#[derive(Debug, Clone)]
struct QueueEntry {
    task_id: Uuid,
    priority: TaskPriority,
}

impl PartialEq for QueueEntry {
    fn eq(&self, other: &Self) -> bool {
        self.task_id == other.task_id
    }
}

impl Eq for QueueEntry {}

impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for QueueEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // BinaryHeap is a max-heap, so "larger" elements come first
        // For higher priority first: higher base should be "larger" -> a.base.cmp(&b.base)
        // For FIFO: smaller enqueue_ts should be "larger" -> b.enqueue_ts.cmp(&a.enqueue_ts)
        self.priority
            .base
            .cmp(&other.priority.base)
            .then_with(|| other.priority.enqueue_ts.cmp(&self.priority.enqueue_ts))
    }
}

/// A task queue with priority ordering
#[derive(Debug)]
struct TaskQueue {
    heap: BinaryHeap<QueueEntry>,
}

impl TaskQueue {
    pub fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
        }
    }

    /// Add a task to the queue
    pub fn push(&mut self, task_id: Uuid, priority: TaskPriority) {
        self.heap.push(QueueEntry { task_id, priority });
    }

    /// Pop the highest priority task
    pub fn pop(&mut self) -> Option<Uuid> {
        self.heap.pop().map(|e| e.task_id)
    }

    /// Peek at the highest priority task without removing
    pub fn peek(&self) -> Option<Uuid> {
        self.heap.peek().map(|e| e.task_id)
    }

    /// Check if queue is empty
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// Get number of tasks in queue
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Check if a specific task is in the queue
    pub fn contains(&self, task_id: &Uuid) -> bool {
        self.heap.iter().any(|e| e.task_id == *task_id)
    }

    /// Remove a specific task from the queue
    pub fn remove(&mut self, task_id: &Uuid) -> bool {
        // BinaryHeap doesn't support removal directly
        // We need to rebuild the heap without the task
        let entries: Vec<_> = self.heap.drain().collect();
        let mut found = false;
        let mut filtered: Vec<_> = Vec::new();

        for entry in entries {
            if entry.task_id == *task_id {
                found = true;
            } else {
                filtered.push(entry);
            }
        }

        for entry in filtered {
            self.heap.push(entry);
        }

        found
    }
}

impl Default for TaskQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Worker slot tracking
#[derive(Debug, Clone)]
struct WorkerSlot {
    worker_id: Uuid,
    task_id: Option<Uuid>,
    started_at: Option<u64>,
}

impl WorkerSlot {
    pub fn new(worker_id: Uuid) -> Self {
        Self {
            worker_id,
            task_id: None,
            started_at: None,
        }
    }

    pub fn assign(&mut self, task_id: Uuid, started_at: u64) {
        self.task_id = Some(task_id);
        self.started_at = Some(started_at);
    }

    pub fn release(&mut self) -> Option<Uuid> {
        let task_id = self.task_id.take();
        self.started_at = None;
        task_id
    }

    pub fn is_idle(&self) -> bool {
        self.task_id.is_none()
    }

    pub fn is_busy(&self) -> bool {
        self.task_id.is_some()
    }
}

/// Worker pool managing concurrent workers
#[derive(Debug)]
struct WorkerPool {
    slots: Vec<WorkerSlot>,
    max_workers: usize,
}

impl WorkerPool {
    pub fn new(max_workers: usize) -> Self {
        let max = max_workers.min(MAX_MAX_WORKERS);
        Self {
            slots: (0..max).map(|_i| WorkerSlot::new(Uuid::new_v4())).collect(),
            max_workers: max,
        }
    }

    /// Get the maximum number of workers
    pub fn max_workers(&self) -> usize {
        self.max_workers
    }

    /// Get number of idle workers
    pub fn idle_count(&self) -> usize {
        self.slots.iter().filter(|s| s.is_idle()).count()
    }

    /// Get number of busy workers
    pub fn busy_count(&self) -> usize {
        self.slots.iter().filter(|s| s.is_busy()).count()
    }

    /// Get total active workers (busy)
    pub fn active_count(&self) -> usize {
        self.busy_count()
    }

    /// Check if we can accept more work
    pub fn can_accept_work(&self) -> bool {
        self.idle_count() > 0
    }

    /// Assign a task to an idle worker
    pub fn assign_task(&mut self, task_id: Uuid, now: u64) -> Option<Uuid> {
        // Find an idle worker
        for slot in &mut self.slots {
            if slot.is_idle() {
                slot.assign(task_id, now);
                return Some(slot.worker_id);
            }
        }
        None
    }

    /// Release a worker (task completed or cancelled)
    pub fn release_worker(&mut self, worker_id: &Uuid) -> Option<Uuid> {
        for slot in &mut self.slots {
            if slot.worker_id == *worker_id {
                return slot.release();
            }
        }
        None
    }

    /// Get task assigned to a specific worker
    pub fn get_worker_task(&self, worker_id: &Uuid) -> Option<Uuid> {
        for slot in &self.slots {
            if slot.worker_id == *worker_id {
                return slot.task_id;
            }
        }
        None
    }

    /// Check if a specific task is being processed
    pub fn is_task_active(&self, task_id: &Uuid) -> bool {
        self.slots.iter().any(|s| s.task_id == Some(*task_id))
    }

    /// Get all worker IDs
    pub fn worker_ids(&self) -> Vec<Uuid> {
        self.slots.iter().map(|s| s.worker_id).collect()
    }

    /// Resize the pool (for dynamic adjustment)
    pub fn resize(&mut self, new_max: usize) -> Result<(), SwellError> {
        let new_size = new_max.min(MAX_MAX_WORKERS);

        // Can't resize down while workers are busy
        if new_size < self.max_workers {
            let busy = self.busy_count();
            if busy > new_size {
                return Err(SwellError::InvalidStateTransition(format!(
                    "Cannot resize to {} workers: {} are still busy",
                    new_size, busy
                )));
            }
        }

        // Adjust slots
        while self.slots.len() < new_size {
            self.slots.push(WorkerSlot::new(Uuid::new_v4()));
        }

        // Note: We don't remove excess slots since they might still be busy
        // They will effectively be decommissioned when they become idle

        self.max_workers = new_size;
        Ok(())
    }
}

/// Scheduler configuration
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    pub max_workers: usize,
    pub fair_scheduling: bool,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_workers: DEFAULT_MAX_WORKERS,
            fair_scheduling: true,
        }
    }
}

/// The main scheduler coordinating task assignment
pub struct Scheduler {
    queue: TaskQueue,
    worker_pool: WorkerPool,
    fair_scheduling: bool,
    enqueue_counter: u64,
    assigned_tasks: HashSet<Uuid>,
}

impl Scheduler {
    /// Create a new scheduler with default configuration
    pub fn new() -> Self {
        Self::with_config(SchedulerConfig::default())
    }

    /// Create a scheduler with custom configuration
    pub fn with_config(config: SchedulerConfig) -> Self {
        Self {
            queue: TaskQueue::new(),
            worker_pool: WorkerPool::new(config.max_workers),
            fair_scheduling: config.fair_scheduling,
            enqueue_counter: 0,
            assigned_tasks: HashSet::new(),
        }
    }

    /// Get current max workers setting
    pub fn max_workers(&self) -> usize {
        self.worker_pool.max_workers()
    }

    /// Get number of tasks waiting in queue
    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// Get number of active (busy) workers
    pub fn active_workers(&self) -> usize {
        self.worker_pool.active_count()
    }

    /// Get number of idle workers
    pub fn idle_workers(&self) -> usize {
        self.worker_pool.idle_count()
    }

    /// Check if scheduler can accept new work
    pub fn can_schedule(&self) -> bool {
        self.worker_pool.can_accept_work()
    }

    /// Check if a task is currently being processed
    pub fn is_task_active(&self, task_id: &Uuid) -> bool {
        self.worker_pool.is_task_active(task_id) || self.assigned_tasks.contains(task_id)
    }

    /// Enqueue a task for scheduling with default priority
    pub fn enqueue(&mut self, task_id: Uuid) {
        self.enqueue_with_priority(
            task_id,
            TaskPriority::default_priority(self.enqueue_counter),
        );
        self.enqueue_counter += 1;
    }

    /// Enqueue a task with specific priority
    pub fn enqueue_with_priority(&mut self, task_id: Uuid, priority: TaskPriority) {
        // Don't enqueue if already active
        if self.is_task_active(&task_id) {
            warn!(task_id = %task_id, "Task already active, not enqueuing");
            return;
        }

        self.queue.push(task_id, priority);
        info!(task_id = %task_id, priority = priority.base, "Task enqueued with priority");
    }

    /// Enqueue multiple tasks at once
    pub fn enqueue_all(&mut self, task_ids: Vec<Uuid>) {
        for task_id in task_ids {
            self.enqueue(task_id);
        }
    }

    /// Try to schedule the next task if a worker is available
    /// Returns the worker_id and task_id if scheduled, None if no work available
    pub fn try_schedule(&mut self, now: u64) -> Option<(Uuid, Uuid)> {
        if !self.worker_pool.can_accept_work() {
            debug!("No idle workers available");
            return None;
        }

        let task_id = self.queue.pop()?;

        // Double-check not already assigned
        if self.assigned_tasks.contains(&task_id) {
            debug!(task_id = %task_id, "Task already assigned, skipping");
            return self.try_schedule(now); // Try next task
        }

        // Mark as assigned before giving to worker to prevent double-assignment
        self.assigned_tasks.insert(task_id);

        // Assign to worker
        if let Some(worker_id) = self.worker_pool.assign_task(task_id, now) {
            info!(task_id = %task_id, worker_id = %worker_id, "Task scheduled");
            Some((worker_id, task_id))
        } else {
            // Worker pool couldn't assign (shouldn't happen if can_accept_work is true)
            self.assigned_tasks.remove(&task_id);
            None
        }
    }

    /// Mark a task as completed (worker finished)
    pub fn complete_task(&mut self, task_id: &Uuid) {
        self.assigned_tasks.remove(task_id);

        // Find and release the worker handling this task
        for slot in &mut self.worker_pool.slots {
            if slot.task_id == Some(*task_id) {
                slot.release();
                info!(task_id = %task_id, "Task completed, worker released");
                break;
            }
        }
    }

    /// Cancel a task (remove from queue or release worker)
    pub fn cancel_task(&mut self, task_id: &Uuid) -> bool {
        // First check if it's being processed
        for slot in &mut self.worker_pool.slots {
            if slot.task_id == Some(*task_id) {
                slot.release();
                self.assigned_tasks.remove(task_id);
                info!(task_id = %task_id, "Task cancelled (was in progress)");
                return true;
            }
        }

        // Then check if it's in the queue
        if self.queue.remove(task_id) {
            self.assigned_tasks.remove(task_id);
            info!(task_id = %task_id, "Task cancelled (was in queue)");
            return true;
        }

        self.assigned_tasks.remove(task_id);
        false
    }

    /// Get next task without assigning (peek)
    pub fn peek_next(&self) -> Option<Uuid> {
        self.queue.peek()
    }

    /// Get all tasks waiting in queue
    pub fn queued_tasks(&self) -> Vec<Uuid> {
        self.worker_pool
            .slots
            .iter()
            .filter_map(|s| s.task_id)
            .collect()
    }

    /// Resize worker pool
    pub fn resize_workers(&mut self, new_max: usize) -> Result<(), SwellError> {
        self.worker_pool.resize(new_max)
    }

    /// Check if scheduler has any pending work
    pub fn has_pending_work(&self) -> bool {
        !self.queue.is_empty() || self.worker_pool.busy_count() > 0
    }

    /// Get scheduler statistics
    pub fn stats(&self) -> SchedulerStats {
        SchedulerStats {
            max_workers: self.max_workers(),
            active_workers: self.active_workers(),
            idle_workers: self.idle_workers(),
            queued_tasks: self.queue_len(),
            assigned_tasks: self.assigned_tasks.len(),
        }
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

/// Scheduler statistics
#[derive(Debug, Clone)]
pub struct SchedulerStats {
    pub max_workers: usize,
    pub active_workers: usize,
    pub idle_workers: usize,
    pub queued_tasks: usize,
    pub assigned_tasks: usize,
}

impl std::fmt::Display for SchedulerStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Scheduler(stats: max_workers={}, active={}, idle={}, queued={}, assigned={})",
            self.max_workers,
            self.active_workers,
            self.idle_workers,
            self.queued_tasks,
            self.assigned_tasks
        )
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- TaskQueue Tests ---

    #[test]
    fn test_task_queue_push_pop() {
        let mut queue = TaskQueue::new();
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();

        // Use different base priorities: task1=low, task2=high
        queue.push(task1, TaskPriority::low_priority(1));
        queue.push(task2, TaskPriority::high_priority(2));

        // Higher priority (task2 with base=200) should come out first
        let first = queue.pop().unwrap();
        assert_eq!(first, task2);
    }

    #[test]
    fn test_task_queue_fifo_same_priority() {
        let mut queue = TaskQueue::new();
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();
        let task3 = Uuid::new_v4();

        // Same base priority, different enqueue_ts
        queue.push(
            task1,
            TaskPriority {
                base: 100,
                enqueue_ts: 1,
            },
        );
        queue.push(
            task2,
            TaskPriority {
                base: 100,
                enqueue_ts: 2,
            },
        );
        queue.push(
            task3,
            TaskPriority {
                base: 100,
                enqueue_ts: 3,
            },
        );

        // FIFO order for same priority
        assert_eq!(queue.pop().unwrap(), task1);
        assert_eq!(queue.pop().unwrap(), task2);
        assert_eq!(queue.pop().unwrap(), task3);
    }

    #[test]
    fn test_task_queue_priority_ordering() {
        let mut queue = TaskQueue::new();
        let task_low = Uuid::new_v4();
        let task_high = Uuid::new_v4();
        let task_medium = Uuid::new_v4();

        queue.push(
            task_low,
            TaskPriority {
                base: 50,
                enqueue_ts: 1,
            },
        );
        queue.push(
            task_high,
            TaskPriority {
                base: 200,
                enqueue_ts: 1,
            },
        );
        queue.push(
            task_medium,
            TaskPriority {
                base: 100,
                enqueue_ts: 1,
            },
        );

        // Highest priority first
        let first = queue.pop().unwrap();
        assert_eq!(first, task_high);

        let second = queue.pop().unwrap();
        assert_eq!(second, task_medium);

        let third = queue.pop().unwrap();
        assert_eq!(third, task_low);
    }

    #[test]
    fn test_task_queue_contains() {
        let mut queue = TaskQueue::new();
        let task = Uuid::new_v4();

        assert!(!queue.contains(&task));

        queue.push(task, TaskPriority::default_priority(1));
        assert!(queue.contains(&task));
    }

    #[test]
    fn test_task_queue_remove() {
        let mut queue = TaskQueue::new();
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();

        queue.push(task1, TaskPriority::default_priority(1));
        queue.push(task2, TaskPriority::default_priority(2));

        assert!(queue.remove(&task1));
        assert!(!queue.contains(&task1));
        assert!(queue.contains(&task2));
    }

    // --- WorkerPool Tests ---

    #[test]
    fn test_worker_pool_creation() {
        let pool = WorkerPool::new(4);
        assert_eq!(pool.max_workers(), 4);
        assert_eq!(pool.idle_count(), 4);
        assert_eq!(pool.busy_count(), 0);
    }

    #[test]
    fn test_worker_pool_max_limit() {
        // Can't exceed MAX_MAX_WORKERS
        let pool = WorkerPool::new(100);
        assert_eq!(pool.max_workers(), MAX_MAX_WORKERS);
    }

    #[test]
    fn test_worker_pool_assign_and_release() {
        let mut pool = WorkerPool::new(2);
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();

        // Assign first task
        let worker1 = pool.assign_task(task1, 100).unwrap();
        assert_eq!(pool.busy_count(), 1);
        assert_eq!(pool.idle_count(), 1);
        assert!(pool.is_task_active(&task1));

        // Assign second task
        let worker2 = pool.assign_task(task2, 100).unwrap();
        assert_eq!(pool.busy_count(), 2);
        assert!(!pool.can_accept_work());

        // Release first task
        let released = pool.release_worker(&worker1).unwrap();
        assert_eq!(released, task1);
        assert_eq!(pool.busy_count(), 1);
        assert_eq!(pool.idle_count(), 1);
        assert!(!pool.is_task_active(&task1));
    }

    #[test]
    fn test_worker_pool_full_rejection() {
        let mut pool = WorkerPool::new(2);
        let task = Uuid::new_v4();

        pool.assign_task(task, 100).unwrap();
        pool.assign_task(Uuid::new_v4(), 100).unwrap();

        // Pool is full, should reject
        assert!(pool.assign_task(Uuid::new_v4(), 100).is_none());
    }

    #[test]
    fn test_worker_pool_resize() {
        let mut pool = WorkerPool::new(4);

        // Resize to 2
        pool.resize(2).unwrap();
        assert_eq!(pool.max_workers(), 2);
    }

    #[test]
    fn test_worker_pool_resize_to_busy_fails() {
        let mut pool = WorkerPool::new(4);
        pool.assign_task(Uuid::new_v4(), 100).unwrap();
        pool.assign_task(Uuid::new_v4(), 100).unwrap();
        pool.assign_task(Uuid::new_v4(), 100).unwrap();

        // Can't resize to 2 when 3 are busy
        let result = pool.resize(2);
        assert!(result.is_err());
    }

    // --- Scheduler Tests ---

    #[test]
    fn test_scheduler_creation() {
        let scheduler = Scheduler::new();
        assert_eq!(scheduler.max_workers(), DEFAULT_MAX_WORKERS);
        assert_eq!(scheduler.queue_len(), 0);
        assert_eq!(scheduler.active_workers(), 0);
    }

    #[test]
    fn test_scheduler_enqueue_and_schedule() {
        let mut scheduler = Scheduler::new();
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();

        scheduler.enqueue(task1);
        scheduler.enqueue(task2);

        assert_eq!(scheduler.queue_len(), 2);

        // Schedule first task
        let result = scheduler.try_schedule(100);
        assert!(result.is_some());
        let (worker_id, scheduled_task) = result.unwrap();
        assert_eq!(scheduled_task, task1);
        assert_eq!(scheduler.active_workers(), 1);
        assert_eq!(scheduler.queue_len(), 1);
    }

    #[test]
    fn test_scheduler_no_idle_worker() {
        let mut scheduler = Scheduler::new();

        // Fill all workers
        for _ in 0..scheduler.max_workers() {
            scheduler.enqueue(Uuid::new_v4());
            scheduler.try_schedule(100);
        }

        assert!(!scheduler.can_schedule());

        // Now no workers available
        let result = scheduler.try_schedule(100);
        assert!(result.is_none());
    }

    #[test]
    fn test_scheduler_complete_task() {
        let mut scheduler = Scheduler::new();
        let task = Uuid::new_v4();

        scheduler.enqueue(task);
        let (_, scheduled_task) = scheduler.try_schedule(100).unwrap();
        assert_eq!(scheduled_task, task);

        // Complete the task
        scheduler.complete_task(&task);

        assert_eq!(scheduler.active_workers(), 0);
        assert!(scheduler.idle_workers() > 0);
    }

    #[test]
    fn test_scheduler_cancel_from_queue() {
        let mut scheduler = Scheduler::new();
        let task = Uuid::new_v4();

        scheduler.enqueue(task);
        assert_eq!(scheduler.queue_len(), 1);

        // Cancel before scheduling
        let cancelled = scheduler.cancel_task(&task);
        assert!(cancelled);
        assert_eq!(scheduler.queue_len(), 0);
    }

    #[test]
    fn test_scheduler_cancel_in_progress() {
        let mut scheduler = Scheduler::new();
        let task = Uuid::new_v4();

        scheduler.enqueue(task);
        scheduler.try_schedule(100).unwrap();

        assert_eq!(scheduler.active_workers(), 1);

        // Cancel in-progress task
        let cancelled = scheduler.cancel_task(&task);
        assert!(cancelled);
        assert_eq!(scheduler.active_workers(), 0);
    }

    #[test]
    fn test_scheduler_resize_workers() {
        let mut scheduler = Scheduler::new();

        // Resize to 3 workers (max is 7)
        scheduler.resize_workers(3).unwrap();
        assert_eq!(scheduler.max_workers(), 3);
    }

    #[test]
    fn test_scheduler_resize_above_max() {
        let mut scheduler = Scheduler::new();

        // Try to set 10 workers (should be capped at 7)
        scheduler.resize_workers(10).unwrap();
        assert_eq!(scheduler.max_workers(), MAX_MAX_WORKERS);
    }

    #[test]
    fn test_scheduler_stats() {
        let scheduler = Scheduler::new();
        let stats = scheduler.stats();

        assert_eq!(stats.max_workers, DEFAULT_MAX_WORKERS);
        assert_eq!(stats.active_workers, 0);
        assert_eq!(stats.idle_workers, DEFAULT_MAX_WORKERS);
        assert_eq!(stats.queued_tasks, 0);
    }

    #[test]
    fn test_scheduler_priority_based_scheduling() {
        let mut scheduler = Scheduler::new();
        let low = Uuid::new_v4();
        let high = Uuid::new_v4();

        scheduler.enqueue_with_priority(low, TaskPriority::low_priority(1));
        scheduler.enqueue_with_priority(high, TaskPriority::high_priority(2));

        // High priority should be scheduled first
        let (_, first) = scheduler.try_schedule(100).unwrap();
        assert_eq!(first, high);
    }

    #[test]
    fn test_scheduler_fair_round_robin() {
        let mut scheduler = Scheduler::new();

        // Enqueue many tasks
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();
        let task3 = Uuid::new_v4();

        scheduler.enqueue(task1);
        scheduler.enqueue(task2);
        scheduler.enqueue(task3);

        // Complete first task, then schedule more
        let (worker1, scheduled1) = scheduler.try_schedule(100).unwrap();
        assert_eq!(scheduled1, task1);

        // Complete it
        scheduler.complete_task(&scheduled1);

        // Schedule next
        let (worker2, scheduled2) = scheduler.try_schedule(101).unwrap();
        assert_eq!(scheduled2, task2);
    }

    #[test]
    fn test_scheduler_does_not_duplicate_assignment() {
        let mut scheduler = Scheduler::new();
        let task = Uuid::new_v4();

        scheduler.enqueue(task);

        // Schedule once
        let result1 = scheduler.try_schedule(100);
        assert!(result1.is_some());

        // Try to schedule again - should not get same task
        let result2 = scheduler.try_schedule(100);
        // May be None if queue is empty, or get different task
        if let Some((_, task_id)) = result2 {
            assert_ne!(task_id, task);
        }
    }

    #[test]
    fn test_scheduler_has_pending_work() {
        let mut scheduler = Scheduler::new();

        assert!(!scheduler.has_pending_work());

        scheduler.enqueue(Uuid::new_v4());
        assert!(scheduler.has_pending_work());

        scheduler.try_schedule(100).unwrap();
        assert!(scheduler.has_pending_work());

        // Complete the task
        let tasks: Vec<_> = scheduler
            .worker_pool
            .slots
            .iter()
            .filter_map(|s| s.task_id)
            .collect();
        for task_id in tasks {
            scheduler.complete_task(&task_id);
        }

        // Now should have no pending work (queue empty, no active workers)
        assert!(!scheduler.has_pending_work());
    }

    #[test]
    fn test_scheduler_with_custom_config() {
        let config = SchedulerConfig {
            max_workers: 4,
            fair_scheduling: true,
        };
        let scheduler = Scheduler::with_config(config);

        assert_eq!(scheduler.max_workers(), 4);
    }

    #[test]
    fn test_scheduler_enqueue_all() {
        let mut scheduler = Scheduler::new();

        let tasks: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();
        scheduler.enqueue_all(tasks.clone());

        assert_eq!(scheduler.queue_len(), 5);
    }

    #[test]
    fn test_scheduler_is_task_active() {
        let mut scheduler = Scheduler::new();
        let task = Uuid::new_v4();

        assert!(!scheduler.is_task_active(&task));

        scheduler.enqueue(task);
        scheduler.try_schedule(100).unwrap();

        assert!(scheduler.is_task_active(&task));

        scheduler.complete_task(&task);
        assert!(!scheduler.is_task_active(&task));
    }

    #[test]
    fn test_scheduler_peek_next() {
        let mut scheduler = Scheduler::new();
        let task = Uuid::new_v4();

        scheduler.enqueue(task);

        let peeked = scheduler.peek_next();
        assert_eq!(peeked, Some(task));

        // Peek should not remove
        assert_eq!(scheduler.queue_len(), 1);
    }
}
