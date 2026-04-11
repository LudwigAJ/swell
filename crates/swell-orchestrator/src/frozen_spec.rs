//! FrozenSpec - Immutable spec reference that cannot be modified once task execution begins.
//!
//! This module provides:
//! - Snapshot of task spec on execution start
//! - Immutable reference during execution
//! - Original spec preservation for audit

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

/// A frozen snapshot of the task specification that cannot be modified during execution.
/// This ensures auditability and prevents mid-execution spec drift.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrozenSpec {
    /// Unique identifier for this frozen spec
    pub id: Uuid,
    /// The task this spec belongs to
    pub task_id: Uuid,
    /// Original task description at snapshot time
    pub description: String,
    /// Original plan at snapshot time (if any)
    pub plan: Option<swell_core::Plan>,
    /// Original scope at snapshot time
    pub scope: swell_core::TaskScope,
    /// Timestamp when spec was frozen (task execution started)
    pub frozen_at: DateTime<Utc>,
    /// Task state when spec was frozen
    pub state_at_freeze: swell_core::TaskState,
}

impl FrozenSpec {
    /// Create a new FrozenSpec by snapshotting the current task state
    pub fn snapshot(task: &swell_core::Task) -> Self {
        Self {
            id: Uuid::new_v4(),
            task_id: task.id,
            description: task.description.clone(),
            plan: task.plan.clone(),
            scope: task.current_scope.clone(),
            frozen_at: Utc::now(),
            state_at_freeze: task.state,
        }
    }

    /// Create a FrozenSpec with explicit values (for testing)
    pub fn new(
        task_id: Uuid,
        description: String,
        plan: Option<swell_core::Plan>,
        scope: swell_core::TaskScope,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            task_id,
            description,
            plan,
            scope,
            frozen_at: Utc::now(),
            state_at_freeze: swell_core::TaskState::Executing,
        }
    }

    /// Get the task ID this spec belongs to
    pub fn task_id(&self) -> Uuid {
        self.task_id
    }

    /// Get the frozen description
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Get a reference to the frozen plan (if any)
    pub fn plan(&self) -> Option<&swell_core::Plan> {
        self.plan.as_ref()
    }

    /// Get the frozen scope
    pub fn scope(&self) -> &swell_core::TaskScope {
        &self.scope
    }

    /// Get the timestamp when spec was frozen
    pub fn frozen_at(&self) -> DateTime<Utc> {
        self.frozen_at
    }

    /// Get the task state when spec was frozen
    pub fn state_at_freeze(&self) -> swell_core::TaskState {
        self.state_at_freeze
    }
}

/// A thread-safe container for a FrozenSpec that ensures immutability.
/// Once frozen, the spec cannot be modified - any attempt to modify it will fail.
#[derive(Debug, Clone)]
pub struct FrozenSpecRef(Arc<FrozenSpec>);

impl FrozenSpecRef {
    /// Create a new FrozenSpecRef from a task snapshot
    pub fn from_task(task: &swell_core::Task) -> Self {
        Self(Arc::new(FrozenSpec::snapshot(task)))
    }

    /// Get a reference to the underlying FrozenSpec
    pub fn get(&self) -> &FrozenSpec {
        &self.0
    }

    /// Get a clone of the inner Arc<FrozenSpec>
    pub fn clone_inner(&self) -> Arc<FrozenSpec> {
        self.0.clone()
    }
}

impl std::ops::Deref for FrozenSpecRef {
    type Target = FrozenSpec;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swell_core::{Plan, PlanStep, RiskLevel, StepStatus, Task, TaskState};

    fn create_test_task() -> Task {
        let mut task = Task::new("Test task description".to_string());
        task.plan = Some(Plan {
            id: Uuid::new_v4(),
            task_id: task.id,
            steps: vec![PlanStep {
                id: Uuid::new_v4(),
                description: "Test step".to_string(),
                affected_files: vec!["test.rs".to_string()],
                expected_tests: vec!["test_foo".to_string()],
                risk_level: RiskLevel::Low,
                dependencies: vec![],
                status: StepStatus::Pending,
            }],
            total_estimated_tokens: 1000,
            risk_assessment: "Low risk".to_string(),
        });
        task.current_scope = swell_core::TaskScope {
            files: vec!["src/lib.rs".to_string()],
            directories: vec!["src/".to_string()],
            allowed_operations: vec!["read".to_string(), "write".to_string()],
        };
        task
    }

    #[test]
    fn test_frozen_spec_snapshot_captures_description() {
        let task = create_test_task();
        let spec = FrozenSpec::snapshot(&task);

        assert_eq!(spec.description, "Test task description");
        assert_eq!(spec.task_id, task.id);
    }

    #[test]
    fn test_frozen_spec_snapshot_captures_plan() {
        let task = create_test_task();
        let spec = FrozenSpec::snapshot(&task);

        assert!(spec.plan.is_some());
        let plan = spec.plan.unwrap();
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].description, "Test step");
    }

    #[test]
    fn test_frozen_spec_snapshot_captures_scope() {
        let task = create_test_task();
        let spec = FrozenSpec::snapshot(&task);

        assert_eq!(spec.scope.files, vec!["src/lib.rs"]);
        assert_eq!(spec.scope.directories, vec!["src/"]);
    }

    #[test]
    fn test_frozen_spec_new_for_testing() {
        let task_id = Uuid::new_v4();
        let spec = FrozenSpec::new(
            task_id,
            "Test description".to_string(),
            None,
            swell_core::TaskScope::default(),
        );

        assert_eq!(spec.task_id, task_id);
        assert_eq!(spec.description, "Test description");
        assert!(spec.plan.is_none());
        assert_eq!(spec.state_at_freeze, TaskState::Executing);
    }

    #[test]
    fn test_frozen_spec_ref_deref() {
        let task = create_test_task();
        let spec_ref = FrozenSpecRef::from_task(&task);

        // Should be able to dereference and access fields
        assert_eq!(spec_ref.description, "Test task description");
        assert_eq!(spec_ref.task_id, task.id);
    }

    #[test]
    fn test_frozen_spec_ref_clone_inner() {
        let task = create_test_task();
        let spec_ref = FrozenSpecRef::from_task(&task);

        // clone_inner should give us an Arc we can clone
        let arc = spec_ref.clone_inner();
        let arc2 = arc.clone();
        assert_eq!(arc.description, arc2.description);
    }

    #[test]
    fn test_frozen_spec_immutability_via_arc() {
        let task = create_test_task();
        let spec_ref = FrozenSpecRef::from_task(&task);

        // The Arc ensures we can't modify the inner data
        // Any attempt to modify would require &mut self
        let frozen = spec_ref.get();
        assert_eq!(frozen.description, "Test task description");

        // We can create multiple references (sharing is safe)
        let frozen2 = spec_ref.get();
        assert_eq!(frozen2.description, frozen.description);
    }

    #[test]
    fn test_frozen_spec_preserves_original_for_audit() {
        let task = create_test_task();
        let spec = FrozenSpec::snapshot(&task);

        // The frozen spec preserves the original description
        // even if the task description were to change (which it can't via FrozenSpec)
        assert_eq!(spec.description(), "Test task description");
        assert_eq!(spec.state_at_freeze(), TaskState::Created);
    }

    #[test]
    fn test_frozen_spec_freeze_timestamp() {
        use chrono::Utc;

        let task = create_test_task();
        let spec = FrozenSpec::snapshot(&task);

        // frozen_at should be set to current time
        let now = Utc::now();
        assert!(spec.frozen_at <= now);
        // And shouldn't be in the future (with small tolerance)
        assert!(spec.frozen_at >= now - chrono::Duration::seconds(1));
    }
}
