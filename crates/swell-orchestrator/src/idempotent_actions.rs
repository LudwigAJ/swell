//! Idempotent action design for safe retry semantics and deduplication.
//!
//! This module provides infrastructure for designing actions that are safe to retry:
//! - **IdempotentAction trait**: Actions that produce the same result regardless of how many times they're executed
//! - **Action deduplication**: Prevents the same action from being executed multiple times concurrently
//! - **Retry semantics**: Safe retry handling with exponential backoff and jitter
//!
//! # Idempotent Action Design
//!
//! An action is idempotent if executing it multiple times produces the same result as executing it once.
//! For example:
//! - "Set task state to EXECUTING" is idempotent (setting a task that's already executing is a no-op)
//! - "Create a file" is NOT idempotent (creating the same file twice will fail or overwrite)
//!
//! # Architecture
//!
//! ```ignore
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        IdempotentAction                          │
//! │  ┌─────────────────┐  ┌──────────────────┐  ┌──────────────────┐  │
//! │  │  execute()      │  │  action_key()   │  │  is_idempotent() │  │
//! │  │  -> Result<T>  │  │  -> ActionKey   │  │  -> bool         │  │
//! │  └─────────────────┘  └──────────────────┘  └──────────────────┘  │
//! └─────────────────────────────────────────────────────────────────┘
//!
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                       ActionDeduplicator                         │
//! │  ┌─────────────────┐  ┌──────────────────┐  ┌──────────────────┐  │
//! │  │  try_execute() │  │  is_running()    │  │  complete()      │  │
//! │  │  -> Result<T>  │  │  -> bool         │  │  -> ()           │  │
//! │  └─────────────────┘  └──────────────────┘  └──────────────────┘  │
//! │  ┌─────────────────────────────────────────────────────────────┐  │
//! │  │  pending_actions: RwLock<HashMap<ActionKey, ActionStatus>> │  │
//! │  └─────────────────────────────────────────────────────────────┘  │
//! └─────────────────────────────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Maximum number of retries for an idempotent action before giving up
pub const MAX_ACTION_RETRIES: u32 = 3;

/// Time window for deduplication (in milliseconds)
/// Actions with the same key within this window are considered duplicates
pub const DEDUP_TIME_WINDOW_MS: i64 = 60_000; // 1 minute

/// Action status tracking
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionStatus {
    /// Action is currently running
    Running,
    /// Action completed successfully
    Completed,
    /// Action failed after all retries
    Failed(String),
    /// Action was cancelled
    Cancelled,
}

/// A unique key identifying an action for deduplication purposes
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct ActionKey {
    /// Task ID this action belongs to (if any)
    pub task_id: Option<Uuid>,
    /// Type of action (e.g., "set_state", "create_task", "assign_agent")
    pub action_type: String,
    /// Unique parameters that distinguish this action from others of the same type
    /// Serialized as a string for easy comparison
    pub parameters: String,
}

impl ActionKey {
    /// Create a new action key
    pub fn new(
        task_id: Option<Uuid>,
        action_type: impl Into<String>,
        parameters: impl Into<String>,
    ) -> Self {
        Self {
            task_id,
            action_type: action_type.into(),
            parameters: parameters.into(),
        }
    }

    /// Create a key for a task-specific action
    pub fn for_task(task_id: Uuid, action_type: impl Into<String>) -> Self {
        Self::new(Some(task_id), action_type, "")
    }

    /// Create a key for a task action with parameters
    pub fn for_task_with_params(
        task_id: Uuid,
        action_type: impl Into<String>,
        params: impl Into<String>,
    ) -> Self {
        Self::new(Some(task_id), action_type, params)
    }
}

/// Metadata about an action execution attempt
#[derive(Debug, Clone)]
pub struct ActionExecution {
    /// Unique execution ID
    pub execution_id: Uuid,
    /// The action key
    pub key: ActionKey,
    /// When the action started
    pub started_at: std::time::Instant,
    /// Number of attempts made
    pub attempts: u32,
    /// Last error if any
    pub last_error: Option<String>,
}

impl ActionExecution {
    /// Create a new action execution tracker
    pub fn new(key: ActionKey) -> Self {
        Self {
            execution_id: Uuid::new_v4(),
            key,
            started_at: std::time::Instant::now(),
            attempts: 0,
            last_error: None,
        }
    }

    /// Record an attempt
    pub fn record_attempt(&mut self, error: Option<String>) {
        self.attempts += 1;
        self.last_error = error;
    }

    /// Check if max retries has been exceeded
    pub fn has_exceeded_max_retries(&self) -> bool {
        self.attempts >= MAX_ACTION_RETRIES
    }

    /// Duration since start
    pub fn duration(&self) -> std::time::Duration {
        self.started_at.elapsed()
    }
}

/// Trait for actions that can be safely executed with idempotent semantics.
///
/// Implementors of this trait guarantee that:
/// - Calling `execute()` multiple times with the same parameters produces the same result
/// - The action is safe to retry if it fails mid-execution
///
/// # Example
///
/// ```ignore
/// struct SetTaskStateAction {
///     task_id: Uuid,
///     new_state: TaskState,
/// }
///
/// impl IdempotentAction for SetTaskStateAction {
///     type Output = ();
///
///     fn action_key(&self) -> ActionKey {
///         ActionKey::for_task_with_params(
///             self.task_id,
///             "set_task_state",
///             format!("{:?}", self.new_state)
///         )
///     }
///
///     fn execute(&self) -> Result<Self::Output, String> {
///         // Actual state transition logic
///         Ok(())
///     }
///
///     fn is_idempotent(&self) -> bool {
///         true // Setting the same state twice is harmless
///     }
/// }
/// ```
pub trait IdempotentAction: Send + Sync {
    /// The output type of the action
    type Output;

    /// Returns a unique key identifying this action for deduplication
    fn action_key(&self) -> ActionKey;

    /// Execute the action and return the result
    fn execute(&self) -> Result<Self::Output, String>;

    /// Returns whether this action is idempotent
    /// If true, the action can be safely retried without side effects
    fn is_idempotent(&self) -> bool;

    /// Optional: Human-readable description for debugging
    fn description(&self) -> Option<String> {
        None
    }
}

/// Status of a tracked action
#[derive(Debug, Clone)]
pub struct TrackedAction {
    /// Action key
    pub key: ActionKey,
    /// Current status
    pub status: ActionStatus,
    /// When the action was first seen
    pub created_at: i64,
    /// When the action was last updated
    pub updated_at: i64,
    /// Execution metadata (if running or completed)
    pub execution: Option<ActionExecution>,
}

impl TrackedAction {
    /// Create a new tracked action
    pub fn new(key: ActionKey) -> Self {
        let now = current_timestamp_ms();
        Self {
            key,
            status: ActionStatus::Running,
            created_at: now,
            updated_at: now,
            execution: None,
        }
    }

    /// Mark as completed
    pub fn mark_completed(&mut self) {
        self.status = ActionStatus::Completed;
        self.updated_at = current_timestamp_ms();
    }

    /// Mark as failed
    pub fn mark_failed(&mut self, error: String) {
        self.status = ActionStatus::Failed(error);
        self.updated_at = current_timestamp_ms();
    }

    /// Mark as cancelled
    pub fn mark_cancelled(&mut self) {
        self.status = ActionStatus::Cancelled;
        self.updated_at = current_timestamp_ms();
    }

    /// Check if this action is still valid (not expired)
    pub fn is_valid(&self, time_window_ms: i64) -> bool {
        let now = current_timestamp_ms();
        (now - self.created_at) < time_window_ms
    }
}

/// Action deduplicator to prevent concurrent execution of the same action.
///
/// The deduplicator ensures that only one instance of an action with a given key
/// can be running at any time. Concurrent attempts to execute the same action
/// will receive a `Err(DuplicateAction)` error.
#[derive(Debug)]
pub struct ActionDeduplicator {
    /// Currently running/completed actions
    actions: RwLock<HashMap<ActionKey, TrackedAction>>,
    /// Time window for deduplication
    time_window_ms: i64,
}

impl ActionDeduplicator {
    /// Create a new action deduplicator with default time window
    pub fn new() -> Self {
        Self::with_time_window(DEDUP_TIME_WINDOW_MS)
    }

    /// Create a new action deduplicator with custom time window
    pub fn with_time_window(time_window_ms: i64) -> Self {
        Self {
            actions: RwLock::new(HashMap::new()),
            time_window_ms,
        }
    }

    /// Try to acquire permission to execute an action.
    ///
    /// Returns:
    /// - `Ok(execution)` if the action can be executed (not a duplicate)
    /// - `Err(DuplicateAction)` if the action is already running or recently completed
    pub async fn try_acquire(&self, key: &ActionKey) -> Result<ActionExecution, DuplicateAction> {
        let mut actions = self.actions.write().await;

        // Check if there's an existing action with this key
        if let Some(existing) = actions.get(key) {
            // Check if it's still valid
            if existing.is_valid(self.time_window_ms) {
                match &existing.status {
                    ActionStatus::Running => {
                        debug!(
                            action_type = %key.action_type,
                            task_id = ?key.task_id,
                            "Duplicate action rejected - already running"
                        );
                        return Err(DuplicateAction::AlreadyRunning {
                            key: key.clone(),
                            started_at: existing.created_at,
                        });
                    }
                    ActionStatus::Completed => {
                        // Completed actions can be re-executed (idempotent semantics)
                        // In a real implementation, we might return cached results
                        // For now, we allow re-execution by updating to Running
                        debug!(
                            action_type = %key.action_type,
                            task_id = ?key.task_id,
                            "Action completed previously, allowing re-execution"
                        );
                        let mut tracked = existing.clone();
                        tracked.status = ActionStatus::Running;
                        tracked.updated_at = current_timestamp_ms();
                        actions.insert(key.clone(), tracked);
                        return Ok(ActionExecution::new(key.clone()));
                    }
                    ActionStatus::Failed(_) | ActionStatus::Cancelled => {
                        // Allow retry of failed/cancelled actions
                        debug!(
                            action_type = %key.action_type,
                            task_id = ?key.task_id,
                            "Previous action was {:?}, allowing retry",
                            existing.status
                        );
                        // Update to running
                        let mut tracked = existing.clone();
                        tracked.status = ActionStatus::Running;
                        tracked.updated_at = current_timestamp_ms();
                        actions.insert(key.clone(), tracked);
                        return Ok(ActionExecution::new(key.clone()));
                    }
                }
            }
        }

        // No existing action - create new tracking
        let tracked = TrackedAction::new(key.clone());
        actions.insert(key.clone(), tracked);

        debug!(
            action_type = %key.action_type,
            task_id = ?key.task_id,
            "Action acquired for execution"
        );

        Ok(ActionExecution::new(key.clone()))
    }

    /// Record a successful completion
    pub async fn complete(&self, key: &ActionKey) {
        let mut actions = self.actions.write().await;

        if let Some(tracked) = actions.get_mut(key) {
            tracked.mark_completed();
            info!(
                action_type = %key.action_type,
                task_id = ?key.task_id,
                "Action completed successfully"
            );
        }
    }

    /// Record a failed action
    pub async fn fail(&self, key: &ActionKey, error: String) {
        let mut actions = self.actions.write().await;

        if let Some(tracked) = actions.get_mut(key) {
            tracked.mark_failed(error.clone());
            warn!(
                action_type = %key.action_type,
                task_id = ?key.task_id,
                error = %error,
                "Action failed"
            );
        }
    }

    /// Record action cancellation
    pub async fn cancel(&self, key: &ActionKey) {
        let mut actions = self.actions.write().await;

        if let Some(tracked) = actions.get_mut(key) {
            tracked.mark_cancelled();
            debug!(
                action_type = %key.action_type,
                task_id = ?key.task_id,
                "Action cancelled"
            );
        }
    }

    /// Check if an action is currently running
    pub async fn is_running(&self, key: &ActionKey) -> bool {
        let actions = self.actions.read().await;

        actions
            .get(key)
            .map(|t| t.status == ActionStatus::Running && t.is_valid(self.time_window_ms))
            .unwrap_or(false)
    }

    /// Get the status of an action
    pub async fn get_status(&self, key: &ActionKey) -> Option<ActionStatus> {
        let actions = self.actions.read().await;

        actions.get(key).map(|t| t.status.clone())
    }

    /// Clean up expired entries
    pub async fn cleanup_expired(&self) {
        let mut actions = self.actions.write().await;
        let now = current_timestamp_ms();

        actions.retain(|_, tracked| {
            let expired = (now - tracked.created_at) >= self.time_window_ms;
            if expired {
                debug!(
                    action_type = %tracked.key.action_type,
                    task_id = ?tracked.key.task_id,
                    "Cleaning up expired action tracking"
                );
            }
            !expired
        });
    }

    /// Get count of active actions
    pub async fn active_count(&self) -> usize {
        let actions = self.actions.read().await;
        actions
            .values()
            .filter(|t| t.status == ActionStatus::Running && t.is_valid(self.time_window_ms))
            .count()
    }
}

impl Default for ActionDeduplicator {
    fn default() -> Self {
        Self::new()
    }
}

/// Error indicating a duplicate action was detected
#[derive(Debug, thiserror::Error)]
pub enum DuplicateAction {
    #[error("Action already running: {key:?} (started at {started_at})")]
    AlreadyRunning { key: ActionKey, started_at: i64 },

    #[error("Action already completed: {key:?} (completed at {completed_at})")]
    AlreadyCompleted { key: ActionKey, completed_at: i64 },
}

/// Result of an idempotent action execution
#[derive(Debug)]
pub enum IdempotentResult<T> {
    /// Action executed successfully
    Success(T),
    /// Action was skipped because a duplicate is running
    SkippedDuplicate { key: ActionKey, running_since: i64 },
    /// Action failed after all retries
    Failed {
        key: ActionKey,
        attempts: u32,
        last_error: String,
    },
}

/// Execute an idempotent action with retry semantics and deduplication.
///
/// This function wraps an idempotent action with:
/// - Deduplication: Prevents concurrent execution of the same action
/// - Retry: Retries failed actions up to MAX_ACTION_RETRIES times
/// - Backoff: Exponential backoff between retries (with jitter)
pub async fn execute_idempotent<A: IdempotentAction>(
    deduplicator: &ActionDeduplicator,
    action: &A,
) -> IdempotentResult<A::Output> {
    let key = action.action_key();
    let is_idempotent = action.is_idempotent();

    // Try to acquire the action (deduplication check)
    match deduplicator.try_acquire(&key).await {
        Err(DuplicateAction::AlreadyRunning { started_at, .. }) => {
            IdempotentResult::SkippedDuplicate {
                key,
                running_since: started_at,
            }
        }
        Err(DuplicateAction::AlreadyCompleted { completed_at, .. }) => {
            // For idempotent actions that already completed, we return success
            // because re-executing would produce the same result.
            // However, we don't actually re-execute - we return SkippedDuplicate
            // and the caller should cache/use the original result.
            // For simplicity, we re-execute only if idempotent.
            if is_idempotent {
                info!(
                    action_type = %key.action_type,
                    "Action already completed and idempotent, re-executing safely"
                );
                // Re-execute to get the result - idempotent means same result
                return match action.execute() {
                    Ok(result) => IdempotentResult::Success(result),
                    Err(e) => IdempotentResult::Failed {
                        key,
                        attempts: 1,
                        last_error: e,
                    },
                };
            }
            IdempotentResult::SkippedDuplicate {
                key,
                running_since: completed_at,
            }
        }
        Ok(_execution) => {
            // Execute the action with retry logic
            let mut attempts = 1u32;

            loop {
                match action.execute() {
                    Ok(result) => {
                        deduplicator.complete(&key).await;
                        return IdempotentResult::Success(result);
                    }
                    Err(e) => {
                        warn!(
                            action_type = %key.action_type,
                            attempt = attempts,
                            error = %e,
                            "Action execution failed"
                        );

                        // Check if we should retry
                        if !is_idempotent {
                            deduplicator.fail(&key, e.clone()).await;
                            return IdempotentResult::Failed {
                                key,
                                attempts,
                                last_error: e,
                            };
                        }

                        if attempts >= MAX_ACTION_RETRIES {
                            deduplicator.fail(&key, e.clone()).await;
                            return IdempotentResult::Failed {
                                key,
                                attempts,
                                last_error: e,
                            };
                        }

                        // Exponential backoff with jitter
                        let backoff_ms = calculate_backoff(attempts);
                        debug!(
                            action_type = %key.action_type,
                            attempt = attempts,
                            backoff_ms = backoff_ms,
                            "Retrying action after backoff"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms as u64))
                            .await;

                        attempts += 1;
                    }
                }
            }
        }
    }
}

/// Calculate exponential backoff with jitter
fn calculate_backoff(attempt: u32) -> i64 {
    // Base delay: 100ms
    // Exponential: 2^attempt
    // Max: 5000ms
    // Add jitter: random 0-25% of base delay

    let base = 100;
    let exponential = 2u64.pow(attempt.min(5));
    let delay_ms = (base * exponential).min(5000) as i64;

    // Add jitter (0-25% of delay)
    let jitter_range = delay_ms / 4;
    let jitter = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
        % jitter_range.max(1);

    delay_ms + jitter
}

/// Get current timestamp in milliseconds
fn current_timestamp_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// Wrapper for making existing closures idempotent
pub struct IdempotentClosure<F, T>
where
    F: Fn() -> Result<T, String> + Send + Sync,
{
    key: ActionKey,
    closure: F,
    is_idempotent: bool,
    description: Option<String>,
}

impl<F, T> IdempotentClosure<F, T>
where
    F: Fn() -> Result<T, String> + Send + Sync,
{
    /// Create a new idempotent closure wrapper
    pub fn new(key: ActionKey, closure: F, is_idempotent: bool) -> Self {
        Self {
            key,
            closure,
            is_idempotent,
            description: None,
        }
    }

    /// Create with description
    pub fn with_description(mut self, desc: String) -> Self {
        self.description = Some(desc);
        self
    }

    /// Set idempotent flag
    pub fn idempotent(mut self, is_idempotent: bool) -> Self {
        self.is_idempotent = is_idempotent;
        self
    }
}

impl<F, T> IdempotentAction for IdempotentClosure<F, T>
where
    F: Fn() -> Result<T, String> + Send + Sync,
{
    type Output = T;

    fn action_key(&self) -> ActionKey {
        self.key.clone()
    }

    fn execute(&self) -> Result<Self::Output, String> {
        (self.closure)()
    }

    fn is_idempotent(&self) -> bool {
        self.is_idempotent
    }

    fn description(&self) -> Option<String> {
        self.description.clone()
    }
}

/// A shareable reference to an ActionDeduplicator
pub type SharedDeduplicator = Arc<ActionDeduplicator>;

/// Create a new shared deduplicator
pub fn create_deduplicator() -> SharedDeduplicator {
    Arc::new(ActionDeduplicator::new())
}

/// Create a deduplicator with custom time window
pub fn create_deduplicator_with_window(time_window_ms: i64) -> SharedDeduplicator {
    Arc::new(ActionDeduplicator::with_time_window(time_window_ms))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- ActionKey Tests ---

    #[test]
    fn test_action_key_creation() {
        let task_id = Uuid::new_v4();
        let key = ActionKey::new(Some(task_id), "test_action", "param1=value1");

        assert_eq!(key.task_id, Some(task_id));
        assert_eq!(key.action_type, "test_action");
        assert_eq!(key.parameters, "param1=value1");
    }

    #[test]
    fn test_action_key_for_task() {
        let task_id = Uuid::new_v4();
        let key = ActionKey::for_task(task_id, "set_state");

        assert_eq!(key.task_id, Some(task_id));
        assert_eq!(key.action_type, "set_state");
        assert_eq!(key.parameters, "");
    }

    #[test]
    fn test_action_key_for_task_with_params() {
        let task_id = Uuid::new_v4();
        let key = ActionKey::for_task_with_params(task_id, "transition", "Executing");

        assert_eq!(key.task_id, Some(task_id));
        assert_eq!(key.action_type, "transition");
        assert_eq!(key.parameters, "Executing");
    }

    #[test]
    fn test_action_key_equality() {
        let task_id = Uuid::new_v4();
        let key1 = ActionKey::for_task(task_id, "test");
        let key2 = ActionKey::for_task(task_id, "test");
        let key3 = ActionKey::for_task(task_id, "other");

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
    }

    #[test]
    fn test_action_key_hash() {
        let task_id = Uuid::new_v4();
        let mut map = HashMap::new();
        let key1 = ActionKey::for_task(task_id, "test");
        let key2 = ActionKey::for_task(task_id, "test");

        map.insert(key1.clone(), "value");

        // key2 should find the same entry
        assert_eq!(map.get(&key2), Some(&"value"));
    }

    // --- ActionExecution Tests ---

    #[test]
    fn test_action_execution_initial() {
        let key = ActionKey::for_task(Uuid::new_v4(), "test");
        let execution = ActionExecution::new(key.clone());

        assert_eq!(execution.key, key);
        assert_eq!(execution.attempts, 0);
        assert!(execution.last_error.is_none());
    }

    #[test]
    fn test_action_execution_record_attempt() {
        let key = ActionKey::for_task(Uuid::new_v4(), "test");
        let mut execution = ActionExecution::new(key);

        execution.record_attempt(Some("Error 1".to_string()));
        assert_eq!(execution.attempts, 1);
        assert_eq!(execution.last_error, Some("Error 1".to_string()));

        execution.record_attempt(Some("Error 2".to_string()));
        assert_eq!(execution.attempts, 2);
        assert_eq!(execution.last_error, Some("Error 2".to_string()));
    }

    #[test]
    fn test_action_execution_exceeded_retries() {
        let key = ActionKey::for_task(Uuid::new_v4(), "test");
        let mut execution = ActionExecution::new(key);

        // Under limit
        execution.attempts = 2;
        assert!(!execution.has_exceeded_max_retries());

        // At limit
        execution.attempts = MAX_ACTION_RETRIES;
        assert!(execution.has_exceeded_max_retries());

        // Over limit
        execution.attempts = MAX_ACTION_RETRIES + 1;
        assert!(execution.has_exceeded_max_retries());
    }

    #[test]
    fn test_action_execution_duration() {
        let key = ActionKey::for_task(Uuid::new_v4(), "test");
        let execution = ActionExecution::new(key);

        // Should be very small initially
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(execution.duration().as_millis() >= 10);
    }

    // --- TrackedAction Tests ---

    #[test]
    fn test_tracked_action_creation() {
        let key = ActionKey::for_task(Uuid::new_v4(), "test");
        let tracked = TrackedAction::new(key.clone());

        assert_eq!(tracked.key, key);
        assert_eq!(tracked.status, ActionStatus::Running);
        assert!(tracked.execution.is_none());
    }

    #[test]
    fn test_tracked_action_mark_completed() {
        let key = ActionKey::for_task(Uuid::new_v4(), "test");
        let mut tracked = TrackedAction::new(key);

        tracked.mark_completed();

        assert_eq!(tracked.status, ActionStatus::Completed);
    }

    #[test]
    fn test_tracked_action_mark_failed() {
        let key = ActionKey::for_task(Uuid::new_v4(), "test");
        let mut tracked = TrackedAction::new(key);

        tracked.mark_failed("Something went wrong".to_string());

        match tracked.status {
            ActionStatus::Failed(msg) => assert_eq!(msg, "Something went wrong"),
            _ => panic!("Expected Failed status"),
        }
    }

    #[test]
    fn test_tracked_action_mark_cancelled() {
        let key = ActionKey::for_task(Uuid::new_v4(), "test");
        let mut tracked = TrackedAction::new(key);

        tracked.mark_cancelled();

        assert_eq!(tracked.status, ActionStatus::Cancelled);
    }

    #[test]
    fn test_tracked_action_is_valid() {
        let key = ActionKey::for_task(Uuid::new_v4(), "test");
        let tracked = TrackedAction::new(key);

        // Within time window
        assert!(tracked.is_valid(60_000));

        // After time window (0ms window)
        assert!(!tracked.is_valid(0));
    }

    // --- ActionDeduplicator Tests ---

    #[tokio::test]
    async fn test_deduplicator_new_action() {
        let deduplicator = ActionDeduplicator::new();
        let key = ActionKey::for_task(Uuid::new_v4(), "test_action");

        let result = deduplicator.try_acquire(&key).await;

        assert!(result.is_ok());
        assert_eq!(deduplicator.active_count().await, 1);
    }

    #[tokio::test]
    async fn test_deduplicator_duplicate_rejected() {
        let deduplicator = ActionDeduplicator::new();
        let key = ActionKey::for_task(Uuid::new_v4(), "test_action");

        // First acquisition succeeds
        let result1 = deduplicator.try_acquire(&key).await;
        assert!(result1.is_ok());

        // Second acquisition fails with AlreadyRunning
        let result2 = deduplicator.try_acquire(&key).await;
        assert!(result2.is_err());

        match result2.unwrap_err() {
            DuplicateAction::AlreadyRunning { .. } => {}
            _ => panic!("Expected AlreadyRunning error"),
        }

        // Still only 1 active
        assert_eq!(deduplicator.active_count().await, 1);
    }

    #[tokio::test]
    async fn test_deduplicator_complete_allows_retry() {
        let deduplicator = ActionDeduplicator::new();
        let key = ActionKey::for_task(Uuid::new_v4(), "test_action");

        // First acquisition
        deduplicator.try_acquire(&key).await.unwrap();

        // Mark as completed
        deduplicator.complete(&key).await;

        // Should be able to retry after completion (for idempotent actions)
        let result = deduplicator.try_acquire(&key).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_deduplicator_fail_allows_retry() {
        let deduplicator = ActionDeduplicator::new();
        let key = ActionKey::for_task(Uuid::new_v4(), "test_action");

        // First acquisition
        deduplicator.try_acquire(&key).await.unwrap();

        // Mark as failed
        deduplicator.fail(&key, "Test error".to_string()).await;

        // Should be able to retry after failure
        let result = deduplicator.try_acquire(&key).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_deduplicator_cancel_allows_retry() {
        let deduplicator = ActionDeduplicator::new();
        let key = ActionKey::for_task(Uuid::new_v4(), "test_action");

        // First acquisition
        deduplicator.try_acquire(&key).await.unwrap();

        // Cancel
        deduplicator.cancel(&key).await;

        // Should be able to retry
        let result = deduplicator.try_acquire(&key).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_deduplicator_is_running() {
        let deduplicator = ActionDeduplicator::new();
        let key = ActionKey::for_task(Uuid::new_v4(), "test_action");

        assert!(!deduplicator.is_running(&key).await);

        deduplicator.try_acquire(&key).await.unwrap();
        assert!(deduplicator.is_running(&key).await);

        deduplicator.complete(&key).await;
        assert!(!deduplicator.is_running(&key).await);
    }

    #[tokio::test]
    async fn test_deduplicator_get_status() {
        let deduplicator = ActionDeduplicator::new();
        let key = ActionKey::for_task(Uuid::new_v4(), "test_action");

        assert!(deduplicator.get_status(&key).await.is_none());

        deduplicator.try_acquire(&key).await.unwrap();
        assert_eq!(
            deduplicator.get_status(&key).await,
            Some(ActionStatus::Running)
        );

        deduplicator.complete(&key).await;
        assert_eq!(
            deduplicator.get_status(&key).await,
            Some(ActionStatus::Completed)
        );
    }

    #[tokio::test]
    async fn test_deduplicator_cleanup_expired() {
        // Create deduplicator with very short window
        let deduplicator = ActionDeduplicator::with_time_window(10); // 10ms
        let key = ActionKey::for_task(Uuid::new_v4(), "test_action");

        deduplicator.try_acquire(&key).await.unwrap();

        // Wait for expiration
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Cleanup should remove it
        deduplicator.cleanup_expired().await;

        // Should be able to acquire again
        let result = deduplicator.try_acquire(&key).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_deduplicator_different_keys_allowed() {
        let deduplicator = ActionDeduplicator::new();
        let task_id = Uuid::new_v4();

        let key1 = ActionKey::for_task(task_id, "action1");
        let key2 = ActionKey::for_task(task_id, "action2");

        let result1 = deduplicator.try_acquire(&key1).await;
        let result2 = deduplicator.try_acquire(&key2).await;

        assert!(result1.is_ok());
        assert!(result2.is_ok());
        assert_eq!(deduplicator.active_count().await, 2);
    }

    // --- Backoff Calculation Tests ---

    #[test]
    fn test_backoff_increases_exponentially() {
        let backoff_1 = calculate_backoff(1);
        let backoff_2 = calculate_backoff(2);
        let backoff_3 = calculate_backoff(3);

        // Each should be larger than the previous (with jitter, so use >)
        assert!(backoff_2 > backoff_1);
        assert!(backoff_3 > backoff_2);
    }

    #[test]
    fn test_backoff_caps_at_max() {
        let backoff_large = calculate_backoff(10);
        assert!(backoff_large <= 5000 + 1250); // max + jitter
    }

    // --- IdempotentClosure Tests ---

    #[tokio::test]
    async fn test_idempotent_closure_execution() {
        let deduplicator = Arc::new(ActionDeduplicator::new());
        let key = ActionKey::for_task(Uuid::new_v4(), "closure_test");
        let call_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));

        let call_count_clone = call_count.clone();
        let closure = IdempotentClosure::new(
            key.clone(),
            move || {
                call_count_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(42)
            },
            true, // idempotent
        );

        let result = execute_idempotent(&deduplicator, &closure).await;

        match result {
            IdempotentResult::Success(value) => assert_eq!(value, 42),
            _ => panic!("Expected Success"),
        }

        assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    // --- SharedDeduplicator Tests ---

    #[tokio::test]
    async fn test_shared_deduplicator() {
        let deduplicator = create_deduplicator();
        let key = ActionKey::for_task(Uuid::new_v4(), "shared_test");

        // Clone and use from different tasks
        let dedup1 = deduplicator.clone();
        let dedup2 = deduplicator.clone();
        let key2 = key.clone();

        let handle1 = tokio::spawn(async move { dedup1.try_acquire(&key).await });

        let handle2 = tokio::spawn(async move {
            // Small delay to ensure handle1 acquires first
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            dedup2.try_acquire(&key2).await
        });

        let result1 = handle1.await.unwrap();
        let result2 = handle2.await.unwrap();

        // One should succeed, one should fail
        let success_count = [result1.is_ok(), result2.is_ok()]
            .iter()
            .filter(|&&v| v)
            .count();

        assert_eq!(success_count, 1);
    }

    // --- Mock IdempotentAction Implementation for Testing ---

    struct MockIdempotentAction {
        key: ActionKey,
        should_fail: bool,
        fail_count: std::sync::atomic::AtomicU32,
    }

    impl MockIdempotentAction {
        fn new(task_id: Uuid, should_fail: bool) -> Self {
            Self {
                key: ActionKey::for_task(task_id, "mock_action"),
                should_fail,
                fail_count: std::sync::atomic::AtomicU32::new(0),
            }
        }
    }

    impl IdempotentAction for MockIdempotentAction {
        type Output = String;

        fn action_key(&self) -> ActionKey {
            self.key.clone()
        }

        fn execute(&self) -> Result<Self::Output, String> {
            if self.should_fail {
                self.fail_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err("Mock failure".to_string())
            } else {
                Ok("Success".to_string())
            }
        }

        fn is_idempotent(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn test_execute_idempotent_success() {
        let deduplicator = create_deduplicator();
        let action = MockIdempotentAction::new(Uuid::new_v4(), false);

        let result = execute_idempotent(&deduplicator, &action).await;

        match result {
            IdempotentResult::Success(value) => assert_eq!(value, "Success"),
            _ => panic!("Expected Success"),
        }
    }

    #[tokio::test]
    async fn test_execute_idempotent_retry_on_failure() {
        let deduplicator = create_deduplicator();
        let action = MockIdempotentAction::new(Uuid::new_v4(), true);

        // This will fail and retry up to MAX_ACTION_RETRIES
        let result = execute_idempotent(&deduplicator, &action).await;

        match result {
            IdempotentResult::Failed { attempts, .. } => {
                assert_eq!(attempts, MAX_ACTION_RETRIES);
            }
            _ => panic!("Expected Failed after retries"),
        }
    }

    #[tokio::test]
    async fn test_execute_idempotent_non_idempotent_no_retry() {
        let deduplicator = create_deduplicator();
        let task_id = Uuid::new_v4();

        struct NonIdempotentAction {
            key: ActionKey,
        }

        impl NonIdempotentAction {
            fn new(task_id: Uuid) -> Self {
                Self {
                    key: ActionKey::for_task(task_id, "non_idempotent"),
                }
            }
        }

        impl IdempotentAction for NonIdempotentAction {
            type Output = ();

            fn action_key(&self) -> ActionKey {
                self.key.clone()
            }

            fn execute(&self) -> Result<Self::Output, String> {
                Err("Non-idempotent failure".to_string())
            }

            fn is_idempotent(&self) -> bool {
                false
            }
        }

        let action = NonIdempotentAction::new(task_id);
        let result = execute_idempotent(&deduplicator, &action).await;

        match result {
            IdempotentResult::Failed { attempts, .. } => {
                // Should fail immediately without retry (non-idempotent)
                assert_eq!(attempts, 1);
            }
            _ => panic!("Expected Failed"),
        }
    }

    // --- Timestamp Tests ---

    #[test]
    fn test_current_timestamp_ms() {
        let ts1 = current_timestamp_ms();
        std::thread::sleep(std::time::Duration::from_millis(1));
        let ts2 = current_timestamp_ms();

        assert!(ts2 > ts1);
    }
}
