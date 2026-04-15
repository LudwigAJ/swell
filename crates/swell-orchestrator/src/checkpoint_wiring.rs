//! Checkpoint wiring module - automatically checkpoints TaskStateMachine transitions.
//!
//! This module provides automatic checkpointing of every state transition in the
//! task lifecycle, enabling full state recovery after process restarts.
//!
//! # Architecture
//!
//! The `CheckpointingTaskStateMachine` wraps a `TaskStateMachine` and automatically
//! saves a checkpoint after every successful state transition. This ensures that
//! task state is recoverable even after a crash or restart.
//!
//! # Example
//!
//! ```ignore
//! use swell_state::traits::in_memory::InMemoryCheckpointStore;
//! use swell_orchestrator::checkpoint_wiring::CheckpointingTaskStateMachine;
//!
//! let store = Arc::new(InMemoryCheckpointStore::new());
//! let sm = CheckpointingTaskStateMachine::new(store);
//!
//! let task = sm.create_task("My task".to_string());
//! sm.enrich_task(task.id).await?;  // Checkpoint saved after transition
//! ```

use std::sync::Arc;

use swell_core::{Checkpoint, CheckpointStore, Plan, SwellError, Task, TaskState};
use tokio::sync::RwLock;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::state_machine::TaskStateMachine;

/// Checkpoint metadata for state transitions
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TransitionMetadata {
    pub previous_state: Option<TaskState>,
    pub new_state: TaskState,
    pub transition_name: String,
    pub checkpoint_index: usize,
}

/// A task state machine that automatically checkpoints every state transition.
///
/// This wrapper ensures that after every successful state transition,
/// a checkpoint is saved containing the full task state. This enables
/// full recovery after process restarts by loading the latest checkpoint.
///
/// # Type Parameters
///
/// - `S`: The underlying checkpoint store (e.g., `InMemoryCheckpointStore`, `SqliteCheckpointStore`)
#[derive(Debug)]
pub struct CheckpointingTaskStateMachine<S: CheckpointStore> {
    /// The underlying task state machine
    inner: TaskStateMachine,
    /// The checkpoint store for persistence
    checkpoint_store: Arc<S>,
    /// Track checkpoint counts per task for metadata
    checkpoint_counts: RwLock<std::collections::HashMap<Uuid, usize>>,
}

impl<S: CheckpointStore> CheckpointingTaskStateMachine<S> {
    /// Create a new CheckpointingTaskStateMachine with the given checkpoint store.
    pub fn new(checkpoint_store: Arc<S>) -> Self {
        Self {
            inner: TaskStateMachine::new(),
            checkpoint_store,
            checkpoint_counts: RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Get a reference to the underlying checkpoint store.
    pub fn checkpoint_store(&self) -> &Arc<S> {
        &self.checkpoint_store
    }

    /// Get the checkpoint count for a task.
    pub async fn checkpoint_count(&self, task_id: Uuid) -> usize {
        let counts = self.checkpoint_counts.read().await;
        counts.get(&task_id).copied().unwrap_or(0)
    }

    /// Save a checkpoint for a task after a state transition.
    async fn save_checkpoint(
        &self,
        task: &Task,
        transition_name: &str,
    ) -> Result<Uuid, SwellError> {
        let task_id = task.id;
        let new_state = task.state;

        // Get the previous checkpoint count for this task
        let mut counts = self.checkpoint_counts.write().await;
        let count = counts.entry(task_id).or_insert(0);
        let checkpoint_index = *count;
        *count += 1;
        drop(counts);

        let metadata = TransitionMetadata {
            previous_state: Some(new_state), // Previous state is stored in checkpoint's state field before transition
            new_state,
            transition_name: transition_name.to_string(),
            checkpoint_index,
        };

        let checkpoint = Checkpoint {
            id: Uuid::new_v4(),
            task_id,
            state: new_state,
            snapshot: serde_json::to_value(task)
                .map_err(|e| SwellError::ConfigError(e.to_string()))?,
            created_at: chrono::Utc::now(),
            metadata: serde_json::to_value(&metadata)
                .map_err(|e| SwellError::ConfigError(e.to_string()))?,
        };

        let id = self.checkpoint_store.save(checkpoint).await?;
        debug!(
            task_id = %task_id,
            checkpoint_id = %id,
            state = ?new_state,
            transition = %transition_name,
            "Checkpoint saved after state transition"
        );
        Ok(id)
    }

    /// Create a new task and checkpoint it.
    pub async fn create_task(&self, description: String) -> Task {
        let task = self.inner.create_task(description);
        if let Err(e) = self.save_checkpoint(&task, "create").await {
            warn!(
                task_id = %task.id,
                error = %e,
                "Failed to checkpoint task creation, continuing anyway"
            );
        }
        task
    }

    /// Get a task by ID.
    pub fn get_task(&self, id: Uuid) -> Result<Task, SwellError> {
        self.inner.get_task(id)
    }

    /// Transition task to ENRICHED state and checkpoint.
    pub async fn enrich_task(&self, id: Uuid) -> Result<(), SwellError> {
        self.inner.enrich_task(id)?;
        let task = self.inner.get_task(id)?;
        self.save_checkpoint(&task, "enrich").await?;
        Ok(())
    }

    /// Transition task to READY state (plan approved) and checkpoint.
    pub async fn ready_task(&self, id: Uuid) -> Result<(), SwellError> {
        self.inner.ready_task(id)?;
        let task = self.inner.get_task(id)?;
        self.save_checkpoint(&task, "ready").await?;
        Ok(())
    }

    /// Assign task to an agent and checkpoint.
    pub async fn assign_task(&self, id: Uuid, agent_id: Uuid) -> Result<(), SwellError> {
        self.inner.assign_task(id, agent_id)?;
        let task = self.inner.get_task(id)?;
        self.save_checkpoint(&task, "assign").await?;
        Ok(())
    }

    /// Start executing the task and checkpoint.
    pub async fn start_execution(&self, id: Uuid) -> Result<(), SwellError> {
        self.inner.start_execution(id)?;
        let task = self.inner.get_task(id)?;
        self.save_checkpoint(&task, "start_execution").await?;
        Ok(())
    }

    /// Start validation phase and checkpoint.
    pub async fn start_validation(&self, id: Uuid) -> Result<(), SwellError> {
        self.inner.start_validation(id)?;
        let task = self.inner.get_task(id)?;
        self.save_checkpoint(&task, "start_validation").await?;
        Ok(())
    }

    /// Mark task as accepted and checkpoint.
    pub async fn accept_task(&self, id: Uuid) -> Result<(), SwellError> {
        self.inner.accept_task(id)?;
        let task = self.inner.get_task(id)?;
        self.save_checkpoint(&task, "accept").await?;
        Ok(())
    }

    /// Mark task as rejected and checkpoint.
    pub async fn reject_task(&self, id: Uuid, reason: String) -> Result<(), SwellError> {
        self.inner.reject_task(id, reason)?;
        let task = self.inner.get_task(id)?;
        self.save_checkpoint(&task, "reject").await?;
        Ok(())
    }

    /// Retry a rejected task and checkpoint.
    pub async fn retry_task(&self, id: Uuid) -> Result<(), SwellError> {
        self.inner.retry_task(id)?;
        let task = self.inner.get_task(id)?;
        self.save_checkpoint(&task, "retry").await?;
        Ok(())
    }

    /// Mark task as failed and checkpoint.
    pub async fn fail_task(&self, id: Uuid) -> Result<(), SwellError> {
        self.inner.fail_task(id)?;
        let task = self.inner.get_task(id)?;
        self.save_checkpoint(&task, "fail").await?;
        Ok(())
    }

    /// Pause a task and checkpoint.
    pub async fn pause_task(&self, id: Uuid, reason: String) -> Result<(), SwellError> {
        self.inner.pause_task(id, reason)?;
        let task = self.inner.get_task(id)?;
        self.save_checkpoint(&task, "pause").await?;
        Ok(())
    }

    /// Resume a paused task and checkpoint.
    pub async fn resume_task(&self, id: Uuid) -> Result<(), SwellError> {
        self.inner.resume_task(id)?;
        let task = self.inner.get_task(id)?;
        self.save_checkpoint(&task, "resume").await?;
        Ok(())
    }

    /// Escalate task and checkpoint.
    pub async fn escalate_task(&self, id: Uuid) -> Result<(), SwellError> {
        self.inner.escalate_task(id)?;
        let task = self.inner.get_task(id)?;
        self.save_checkpoint(&task, "escalate").await?;
        Ok(())
    }

    /// Set plan for task (no checkpoint needed for plan setting alone).
    pub fn set_plan(&self, id: Uuid, plan: Plan) -> Result<(), SwellError> {
        self.inner.set_plan(id, plan)
    }

    /// Inject instructions into a task.
    pub fn inject_instruction(&self, id: Uuid, instruction: String) -> Result<(), SwellError> {
        self.inner.inject_instruction(id, instruction)
    }

    /// Modify task scope boundaries.
    pub fn modify_scope(
        &self,
        id: Uuid,
        new_scope: swell_core::TaskScope,
    ) -> Result<(), SwellError> {
        self.inner.modify_scope(id, new_scope)
    }

    /// Restore original scope.
    pub fn restore_original_scope(&self, id: Uuid) -> Result<(), SwellError> {
        self.inner.restore_original_scope(id)
    }

    /// Check if task can proceed (dependencies satisfied).
    pub fn can_proceed(&self, id: Uuid) -> Result<bool, SwellError> {
        self.inner.can_proceed(id)
    }

    /// Get all tasks in a specific state.
    pub fn get_tasks_by_state(&self, state: TaskState) -> Vec<Task> {
        self.inner.get_tasks_by_state(state)
    }

    /// Get all tasks.
    pub fn get_all_tasks(&self) -> Vec<Task> {
        self.inner.get_all_tasks()
    }

    /// Upsert a task directly.
    pub fn upsert_task(&self, task: Task) {
        self.inner.upsert_task(task);
    }

    /// Remove a task from the registry.
    pub fn remove_task(&self, id: Uuid) -> Option<Task> {
        self.inner.remove_task(id)
    }

    /// Load the latest checkpoint for a task and restore it.
    pub async fn restore_from_latest(&self, task_id: Uuid) -> Result<Option<Task>, SwellError> {
        let checkpoint = self.checkpoint_store.load_latest(task_id).await?;

        if let Some(cp) = checkpoint {
            let task: Task = serde_json::from_value(cp.snapshot)
                .map_err(|e| SwellError::ConfigError(e.to_string()))?;

            // Upsert the task into the state machine
            self.inner.upsert_task(task.clone());

            debug!(
                task_id = %task_id,
                state = ?task.state,
                "Task restored from checkpoint"
            );

            Ok(Some(task))
        } else {
            Ok(None)
        }
    }

    /// List all checkpoints for a task.
    pub async fn list_checkpoints(&self, task_id: Uuid) -> Result<Vec<Checkpoint>, SwellError> {
        self.checkpoint_store.list(task_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swell_state::traits::in_memory::InMemoryCheckpointStore;

    /// Helper to create a test plan for a task.
    fn create_test_plan(task_id: Uuid) -> Plan {
        Plan {
            id: Uuid::new_v4(),
            task_id,
            steps: vec![swell_core::PlanStep {
                id: Uuid::new_v4(),
                description: "Test step".to_string(),
                affected_files: vec!["test.rs".to_string()],
                expected_tests: vec!["test_foo".to_string()],
                risk_level: swell_core::RiskLevel::Low,
                dependencies: vec![],
                status: swell_core::StepStatus::Pending,
            }],
            total_estimated_tokens: 1000,
            risk_assessment: "Low risk".to_string(),
        }
    }

    #[tokio::test]
    async fn test_checkpoint_on_create() {
        let store = Arc::new(InMemoryCheckpointStore::new());
        let sm = CheckpointingTaskStateMachine::new(store.clone());

        let task = sm.create_task("Test task".to_string()).await;

        // Verify checkpoint was saved
        let checkpoints = sm.list_checkpoints(task.id).await.unwrap();
        assert_eq!(checkpoints.len(), 1);
        assert_eq!(checkpoints[0].state, TaskState::Created);
    }

    #[tokio::test]
    async fn test_checkpoint_on_enrich() {
        let store = Arc::new(InMemoryCheckpointStore::new());
        let sm = CheckpointingTaskStateMachine::new(store.clone());

        let task = sm.create_task("Test task".to_string()).await;
        sm.enrich_task(task.id).await.unwrap();

        // Verify checkpoints: create + enrich
        let checkpoints = sm.list_checkpoints(task.id).await.unwrap();
        assert_eq!(checkpoints.len(), 2);
        assert_eq!(checkpoints[0].state, TaskState::Created);
        assert_eq!(checkpoints[1].state, TaskState::Enriched);
    }

    #[tokio::test]
    async fn test_checkpoint_on_ready() {
        let store = Arc::new(InMemoryCheckpointStore::new());
        let sm = CheckpointingTaskStateMachine::new(store.clone());

        let task = sm.create_task("Test task".to_string()).await;
        sm.set_plan(task.id, create_test_plan(task.id)).unwrap();
        sm.enrich_task(task.id).await.unwrap();
        sm.ready_task(task.id).await.unwrap();

        // Verify checkpoints: create + enrich + ready
        let checkpoints = sm.list_checkpoints(task.id).await.unwrap();
        assert_eq!(checkpoints.len(), 3);
        assert_eq!(checkpoints[2].state, TaskState::Ready);
    }

    #[tokio::test]
    async fn test_checkpoint_on_assign() {
        let store = Arc::new(InMemoryCheckpointStore::new());
        let sm = CheckpointingTaskStateMachine::new(store.clone());

        let task = sm.create_task("Test task".to_string()).await;
        sm.set_plan(task.id, create_test_plan(task.id)).unwrap();
        sm.enrich_task(task.id).await.unwrap();
        sm.ready_task(task.id).await.unwrap();

        let agent_id = Uuid::new_v4();
        sm.assign_task(task.id, agent_id).await.unwrap();

        // Verify checkpoints: create + enrich + ready + assign
        let checkpoints = sm.list_checkpoints(task.id).await.unwrap();
        assert_eq!(checkpoints.len(), 4);
        assert_eq!(checkpoints[3].state, TaskState::Assigned);
    }

    #[tokio::test]
    async fn test_checkpoint_on_start_execution() {
        let store = Arc::new(InMemoryCheckpointStore::new());
        let sm = CheckpointingTaskStateMachine::new(store.clone());

        let task = sm.create_task("Test task".to_string()).await;
        sm.set_plan(task.id, create_test_plan(task.id)).unwrap();
        sm.enrich_task(task.id).await.unwrap();
        sm.ready_task(task.id).await.unwrap();
        sm.assign_task(task.id, Uuid::new_v4()).await.unwrap();
        sm.start_execution(task.id).await.unwrap();

        // Verify checkpoints: create + enrich + ready + assign + start_execution
        let checkpoints = sm.list_checkpoints(task.id).await.unwrap();
        assert_eq!(checkpoints.len(), 5);
        assert_eq!(checkpoints[4].state, TaskState::Executing);
    }

    #[tokio::test]
    async fn test_checkpoint_on_start_validation() {
        let store = Arc::new(InMemoryCheckpointStore::new());
        let sm = CheckpointingTaskStateMachine::new(store.clone());

        let task = sm.create_task("Test task".to_string()).await;
        sm.set_plan(task.id, create_test_plan(task.id)).unwrap();
        sm.enrich_task(task.id).await.unwrap();
        sm.ready_task(task.id).await.unwrap();
        sm.assign_task(task.id, Uuid::new_v4()).await.unwrap();
        sm.start_execution(task.id).await.unwrap();
        sm.start_validation(task.id).await.unwrap();

        // Verify checkpoints: create + enrich + ready + assign + start_execution + start_validation
        let checkpoints = sm.list_checkpoints(task.id).await.unwrap();
        assert_eq!(checkpoints.len(), 6);
        assert_eq!(checkpoints[5].state, TaskState::Validating);
    }

    #[tokio::test]
    async fn test_checkpoint_on_accept() {
        let store = Arc::new(InMemoryCheckpointStore::new());
        let sm = CheckpointingTaskStateMachine::new(store.clone());

        let task = sm.create_task("Test task".to_string()).await;
        sm.set_plan(task.id, create_test_plan(task.id)).unwrap();
        sm.enrich_task(task.id).await.unwrap();
        sm.ready_task(task.id).await.unwrap();
        sm.assign_task(task.id, Uuid::new_v4()).await.unwrap();
        sm.start_execution(task.id).await.unwrap();
        sm.start_validation(task.id).await.unwrap();
        sm.accept_task(task.id).await.unwrap();

        // Verify checkpoints: 7 transitions
        let checkpoints = sm.list_checkpoints(task.id).await.unwrap();
        assert_eq!(checkpoints.len(), 7);
        assert_eq!(checkpoints[6].state, TaskState::Accepted);
    }

    #[tokio::test]
    async fn test_checkpoint_on_reject() {
        let store = Arc::new(InMemoryCheckpointStore::new());
        let sm = CheckpointingTaskStateMachine::new(store.clone());

        let task = sm.create_task("Test task".to_string()).await;
        sm.set_plan(task.id, create_test_plan(task.id)).unwrap();
        sm.enrich_task(task.id).await.unwrap();
        sm.ready_task(task.id).await.unwrap();
        sm.assign_task(task.id, Uuid::new_v4()).await.unwrap();
        sm.start_execution(task.id).await.unwrap();
        sm.start_validation(task.id).await.unwrap();
        sm.reject_task(task.id, "Test rejection".to_string())
            .await
            .unwrap();

        // Verify checkpoints: 7 transitions
        let checkpoints = sm.list_checkpoints(task.id).await.unwrap();
        assert_eq!(checkpoints.len(), 7);
        assert_eq!(checkpoints[6].state, TaskState::Rejected);
    }

    #[tokio::test]
    async fn test_state_recovery_after_simulated_restart() {
        let store = Arc::new(InMemoryCheckpointStore::new());
        let sm = CheckpointingTaskStateMachine::new(store.clone());

        // Create task and transition through full lifecycle to Accepted
        let task = sm.create_task("Test task".to_string()).await;
        let task_id = task.id;
        sm.set_plan(task_id, create_test_plan(task_id)).unwrap();
        sm.enrich_task(task_id).await.unwrap();
        sm.ready_task(task_id).await.unwrap();
        sm.assign_task(task_id, Uuid::new_v4()).await.unwrap();
        sm.start_execution(task_id).await.unwrap();
        sm.start_validation(task_id).await.unwrap();
        sm.accept_task(task_id).await.unwrap();

        // Simulate restart by creating a new state machine instance with the same store
        let sm2 = CheckpointingTaskStateMachine::new(store.clone());

        // Restore from latest checkpoint
        let restored_task = sm2.restore_from_latest(task_id).await.unwrap().unwrap();

        // Verify the restored task has the correct state
        assert_eq!(restored_task.id, task_id);
        assert_eq!(restored_task.state, TaskState::Accepted);
        assert_eq!(restored_task.description, "Test task");
    }

    #[tokio::test]
    async fn test_full_lifecycle_checkpoints() {
        let store = Arc::new(InMemoryCheckpointStore::new());
        let sm = CheckpointingTaskStateMachine::new(store.clone());

        // Transition through: Created → Enriched → Ready → Assigned → Executing → Validating → Accepted
        let task = sm.create_task("Full lifecycle test".to_string()).await;
        let task_id = task.id;

        sm.set_plan(task_id, create_test_plan(task_id)).unwrap();
        sm.enrich_task(task_id).await.unwrap();
        sm.ready_task(task_id).await.unwrap();
        sm.assign_task(task_id, Uuid::new_v4()).await.unwrap();
        sm.start_execution(task_id).await.unwrap();
        sm.start_validation(task_id).await.unwrap();
        sm.accept_task(task_id).await.unwrap();

        // We should have 7 checkpoints (create + enrich + ready + assign + start_execution + start_validation + accept)
        let checkpoints = sm.list_checkpoints(task_id).await.unwrap();
        assert_eq!(
            checkpoints.len(),
            7,
            "Expected 7 checkpoints for full lifecycle, got {}",
            checkpoints.len()
        );

        // Verify checkpoint count matches transitions
        let sm_count = sm.checkpoint_count(task_id).await;
        assert_eq!(sm_count, 7);

        // Verify final state is Accepted
        let final_task = sm.get_task(task_id).unwrap();
        assert_eq!(final_task.state, TaskState::Accepted);
    }

    #[tokio::test]
    async fn test_checkpoint_metadata_contains_transition_info() {
        let store = Arc::new(InMemoryCheckpointStore::new());
        let sm = CheckpointingTaskStateMachine::new(store.clone());

        let task = sm.create_task("Metadata test".to_string()).await;
        sm.set_plan(task.id, create_test_plan(task.id)).unwrap();
        sm.enrich_task(task.id).await.unwrap();

        let checkpoints = sm.list_checkpoints(task.id).await.unwrap();

        // Check that metadata contains transition information
        let create_meta: TransitionMetadata =
            serde_json::from_value(checkpoints[0].metadata.clone()).unwrap();
        assert_eq!(create_meta.transition_name, "create");
        assert_eq!(create_meta.checkpoint_index, 0);

        let enrich_meta: TransitionMetadata =
            serde_json::from_value(checkpoints[1].metadata.clone()).unwrap();
        assert_eq!(enrich_meta.transition_name, "enrich");
        assert_eq!(enrich_meta.checkpoint_index, 1);
    }

    #[tokio::test]
    async fn test_checkpoint_includes_task_id_state_and_metadata() {
        let store = Arc::new(InMemoryCheckpointStore::new());
        let sm = CheckpointingTaskStateMachine::new(store.clone());

        let task = sm.create_task("Snapshot test".to_string()).await;
        sm.enrich_task(task.id).await.unwrap();

        let checkpoints = sm.list_checkpoints(task.id).await.unwrap();
        let checkpoint = &checkpoints[1]; // enrich checkpoint

        // Verify checkpoint contains task_id
        assert_eq!(checkpoint.task_id, task.id);

        // Verify checkpoint contains state
        assert_eq!(checkpoint.state, TaskState::Enriched);

        // Verify snapshot is a valid Task JSON
        let snapshot: Task = serde_json::from_value(checkpoint.snapshot.clone()).unwrap();
        assert_eq!(snapshot.id, task.id);
        assert_eq!(snapshot.state, TaskState::Enriched);

        // Verify metadata is valid JSON
        let metadata: TransitionMetadata =
            serde_json::from_value(checkpoint.metadata.clone()).unwrap();
        assert_eq!(metadata.new_state, TaskState::Enriched);
    }

    #[tokio::test]
    async fn test_recovery_from_checkpoint_restores_all_fields() {
        let store = Arc::new(InMemoryCheckpointStore::new());
        let sm = CheckpointingTaskStateMachine::new(store.clone());

        let task = sm.create_task("Recovery test".to_string()).await;
        let task_id = task.id;
        sm.set_plan(task_id, create_test_plan(task_id)).unwrap();
        sm.enrich_task(task_id).await.unwrap();
        sm.ready_task(task_id).await.unwrap();
        sm.assign_task(task_id, Uuid::new_v4()).await.unwrap();
        sm.start_execution(task_id).await.unwrap();

        // Verify the task has assigned_agent set
        let task_before = sm.get_task(task_id).unwrap();
        assert!(task_before.assigned_agent.is_some());

        // Simulate restart
        let sm2 = CheckpointingTaskStateMachine::new(store.clone());
        let restored = sm2.restore_from_latest(task_id).await.unwrap().unwrap();

        // Verify all fields are restored
        assert_eq!(restored.id, task_id);
        assert_eq!(restored.state, TaskState::Executing);
        assert!(restored.assigned_agent.is_some());
        assert!(restored.plan.is_some());
    }
}
