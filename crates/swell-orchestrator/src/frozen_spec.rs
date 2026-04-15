//! FrozenSpec - Immutable spec reference that cannot be modified once task execution begins.
//!
//! This module provides:
//! - Snapshot of task spec on execution start
//! - Immutable reference during execution
//! - Original spec preservation for audit
//! - Requirement registry for traceability verification
//!
//! # VAL-ORCH-009: Frozen spec rejects tasks that don't trace to original requirements
//!
//! Every proposed task must trace to at least one requirement in the frozen spec.
//! Tasks that cannot be traced are rejected with a clear reason indicating
//! no matching requirement was found.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info};
use uuid::Uuid;

/// A requirement that a task can trace to for acceptance.
/// Requirements are stored in the frozen spec and used for traceability verification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SpecRequirement {
    /// Unique identifier for the requirement (e.g., "REQ-001")
    pub id: String,
    /// Human-readable description of the requirement
    pub description: String,
    /// Keywords that indicate task may relate to this requirement
    keywords: Vec<String>,
}

impl SpecRequirement {
    /// Create a new spec requirement with keywords for traceability matching
    pub fn new(id: impl Into<String>, description: impl Into<String>, keywords: Vec<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            keywords,
        }
    }

    /// Check if this requirement matches a task description
    /// Returns true if any keyword appears in the task description (case-insensitive)
    pub fn matches_task(&self, task_description: &str) -> bool {
        let task_lower = task_description.to_lowercase();
        self.keywords.iter().any(|kw| {
            let kw_lower = kw.to_lowercase();
            task_lower.contains(&kw_lower)
        })
    }
}

/// Result of verifying task traceability against the frozen spec
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceabilityResult {
    /// Whether the task traces to at least one requirement
    pub is_traced: bool,
    /// The requirement ID that was matched, if any
    pub matched_requirement: Option<String>,
    /// Reason for acceptance or rejection
    pub reason: String,
}

impl TraceabilityResult {
    /// Create an accepted result (task traces to a requirement)
    pub fn traced(requirement_id: &str) -> Self {
        Self {
            is_traced: true,
            matched_requirement: Some(requirement_id.to_string()),
            reason: format!("Task traces to requirement {}", requirement_id),
        }
    }

    /// Create a rejected result (task doesn't trace to any requirement)
    pub fn not_traced() -> Self {
        Self {
            is_traced: false,
            matched_requirement: None,
            reason: "no matching requirement".to_string(),
        }
    }
}

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

/// Immutable requirement registry that cannot be modified after initialization.
/// This ensures that requirements used for traceability verification are stable.
#[derive(Debug, Clone)]
pub struct FrozenRequirementRegistry {
    /// The requirements that this registry manages
    requirements: Vec<SpecRequirement>,
    /// Index for fast lookup by ID
    requirement_ids: std::collections::HashMap<String, usize>,
}

impl FrozenRequirementRegistry {
    /// Create a new registry with the given requirements.
    /// Requirements cannot be added, removed, or modified after creation.
    pub fn new(requirements: Vec<SpecRequirement>) -> Self {
        let mut requirement_ids = std::collections::HashMap::new();
        for (i, req) in requirements.iter().enumerate() {
            requirement_ids.insert(req.id.clone(), i);
        }

        Self {
            requirements,
            requirement_ids,
        }
    }

    /// Create a registry with default requirements for testing
    pub fn with_default_requirements() -> Self {
        let requirements = vec![
            SpecRequirement::new(
                "REQ-001",
                "Implement core task state machine",
                vec!["state machine".to_string(), "task state".to_string(), "created".to_string(), "executing".to_string()],
            ),
            SpecRequirement::new(
                "REQ-002",
                "Implement task planning and scheduling",
                vec!["planning".to_string(), "scheduler".to_string(), "plan".to_string(), "schedule".to_string()],
            ),
            SpecRequirement::new(
                "REQ-003",
                "Implement task validation and acceptance",
                vec!["validation".to_string(), "acceptance".to_string(), "test".to_string(), "lint".to_string()],
            ),
        ];
        Self::new(requirements)
    }

    /// Verify if a task description traces to any requirement in the frozen spec.
    /// Returns a TraceabilityResult indicating whether the task was accepted or rejected.
    ///
    /// # VAL-ORCH-009
    /// - Tasks tracing to a requirement are accepted
    /// - Tasks without traceability are rejected with "no matching requirement" reason
    pub fn verify_traceability(&self, task_description: &str) -> TraceabilityResult {
        debug!(task_description = %task_description, "Verifying task traceability against frozen spec");

        for req in &self.requirements {
            if req.matches_task(task_description) {
                info!(
                    requirement_id = %req.id,
                    task_description = %task_description,
                    "Task traces to requirement"
                );
                return TraceabilityResult::traced(&req.id);
            }
        }

        debug!(
            task_description = %task_description,
            "Task does not trace to any requirement"
        );
        TraceabilityResult::not_traced()
    }

    /// Get a requirement by its ID, if it exists
    pub fn get_requirement(&self, id: &str) -> Option<&SpecRequirement> {
        self.requirements
            .get(*self.requirement_ids.get(id)?)
    }

    /// Get all requirements
    pub fn requirements(&self) -> &[SpecRequirement] {
        &self.requirements
    }

    /// Get the number of requirements
    pub fn len(&self) -> usize {
        self.requirements.len()
    }

    /// Check if there are no requirements
    pub fn is_empty(&self) -> bool {
        self.requirements.is_empty()
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

    // ============================================================================
    // VAL-ORCH-009: Frozen spec rejects tasks that don't trace to original requirements
    // ============================================================================

    #[test]
    fn test_val_orch_009_load_frozen_spec_with_3_requirements() {
        // Test that the frozen spec can be loaded with 3 requirements
        let registry = FrozenRequirementRegistry::with_default_requirements();

        // Verify we have exactly 3 requirements
        assert_eq!(registry.len(), 3);

        // Verify each requirement has the expected ID
        assert!(registry.get_requirement("REQ-001").is_some());
        assert!(registry.get_requirement("REQ-002").is_some());
        assert!(registry.get_requirement("REQ-003").is_some());
    }

    #[test]
    fn test_val_orch_009_task_tracing_to_requirement_2_is_accepted() {
        // Test that a task tracing to requirement #2 is accepted
        let registry = FrozenRequirementRegistry::with_default_requirements();

        // Task that mentions "planning" should trace to REQ-002
        let result = registry.verify_traceability("Implement task planning for scheduling");
        assert!(
            result.is_traced,
            "Task mentioning 'planning' should trace to a requirement"
        );
        assert_eq!(
            result.matched_requirement,
            Some("REQ-002".to_string()),
            "Task mentioning 'planning' should match REQ-002"
        );
    }

    #[test]
    fn test_val_orch_009_task_without_traceability_is_rejected() {
        // Test that a task without traceability is rejected with "no matching requirement"
        let registry = FrozenRequirementRegistry::with_default_requirements();

        // Task that has nothing to do with any requirement
        let result = registry.verify_traceability("Buy groceries for dinner");
        assert!(
            !result.is_traced,
            "Task unrelated to any requirement should be rejected"
        );
        assert_eq!(
            result.reason, "no matching requirement",
            "Rejection reason should be 'no matching requirement'"
        );
        assert_eq!(result.matched_requirement, None);
    }

    #[test]
    fn test_val_orch_009_frozen_registry_cannot_be_mutated() {
        // Test that the FrozenRequirementRegistry is truly immutable
        // by verifying there is no mutating method available
        let registry = FrozenRequirementRegistry::with_default_requirements();

        // Get immutable reference
        let _immutable_ref = &registry;

        // The registry has no methods that can modify its requirements
        // This is enforced by the type system - there is no `pub fn add_requirement`
        // or `pub fn remove_requirement` or any mutable method

        // We can only verify traceability, get requirements, or get length
        assert_eq!(registry.len(), 3);
        let _reqs = registry.requirements();
        let _req = registry.get_requirement("REQ-001");
        let _result = registry.verify_traceability("test");
    }

    #[test]
    fn test_spec_requirement_matches_task_case_insensitive() {
        // Test that requirement matching is case-insensitive
        let req = SpecRequirement::new("TEST-1", "Test requirement", vec!["PLANNING".to_string()]);

        assert!(req.matches_task("implement planning features"));
        assert!(req.matches_task("IMPLEMENT PLANNING FEATURES"));
        assert!(req.matches_task("Planning is important"));
        // Non-matching
        assert!(!req.matches_task("implement validation features"));
    }

    #[test]
    fn test_traceability_result_factory_methods() {
        // Test TraceabilityResult factory methods
        let traced = TraceabilityResult::traced("REQ-001");
        assert!(traced.is_traced);
        assert_eq!(traced.matched_requirement, Some("REQ-001".to_string()));

        let not_traced = TraceabilityResult::not_traced();
        assert!(!not_traced.is_traced);
        assert_eq!(not_traced.matched_requirement, None);
        assert_eq!(not_traced.reason, "no matching requirement");
    }

    #[test]
    fn test_frozen_registry_with_custom_requirements() {
        // Test creating a registry with custom requirements
        let requirements = vec![
            SpecRequirement::new("CUSTOM-1", "Custom requirement 1", vec!["custom".to_string()]),
            SpecRequirement::new("CUSTOM-2", "Custom requirement 2", vec!["special".to_string()]),
        ];
        let registry = FrozenRequirementRegistry::new(requirements);

        assert_eq!(registry.len(), 2);

        // Verify custom matching works
        let result = registry.verify_traceability("This is a custom task");
        assert!(result.is_traced);
        assert_eq!(result.matched_requirement, Some("CUSTOM-1".to_string()));

        // Non-matching
        let result = registry.verify_traceability("This is a random task");
        assert!(!result.is_traced);
    }
}
