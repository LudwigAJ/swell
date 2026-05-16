//! Built-in `FollowUpProposerTrigger` — PR 12 follow-up slice of
//! `plan/flow_integration_plan/12_task_generation_failure_and_followup.md`.
//!
//! Fires on [`Stage::AfterTask`] on the success path: when the task's
//! [`TaskValidationResult`][swell_validation::orchestrator::TaskValidationResult]
//! shows `passed = true`, this trigger asks
//! [`FollowUpGenerator`][crate::followup_generator::FollowUpGenerator] for
//! the implied work that follows from the accepted task (warnings to
//! address, documentation gaps when a public API file was touched, test
//! coverage suggestions, etc.) and pushes each proposal into the live
//! [`ProposalQueue`][crate::proposal_queue::ProposalQueue].
//!
//! Proposals are **not** automatically promoted to executable tasks — they
//! sit in `Pending` until an operator (or, once PR `13` lands, the
//! autonomy-gate machinery) calls
//! [`ProposalQueue::approve`][crate::proposal_queue::ProposalQueue::approve].
//!
//! The trigger captures a `Weak<Orchestrator>` at factory time the same
//! way `FailureExtractionTrigger` does so it can resolve the live queue at
//! fire time without holding a strong cycle.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use tracing::{info, warn};

use crate::followup_generator::{FollowUpGenerator, FollowUpGeneratorConfig};
use crate::trigger_config::TriggerFactoryRegistry;
use crate::triggers::{Stage, Trigger, TriggerContext, TriggerOutcome};
use crate::Orchestrator;

/// `AfterTask` success-path trigger that enqueues follow-up proposals.
pub struct FollowUpProposerTrigger {
    stages: &'static [Stage],
    orchestrator: Weak<Orchestrator>,
    generator: FollowUpGenerator,
}

impl FollowUpProposerTrigger {
    pub fn new(
        stages: &'static [Stage],
        orchestrator: Weak<Orchestrator>,
        generator: FollowUpGenerator,
    ) -> Self {
        Self {
            stages,
            orchestrator,
            generator,
        }
    }

    /// Convenience constructor for the default `AfterTask`-only wiring
    /// with the generator's default config.
    pub fn after_task(orchestrator: Weak<Orchestrator>) -> Self {
        Self::new(&[Stage::AfterTask], orchestrator, FollowUpGenerator::new())
    }
}

#[async_trait]
impl Trigger for FollowUpProposerTrigger {
    fn name(&self) -> &'static str {
        "followup_proposer"
    }

    fn stages(&self) -> &'static [Stage] {
        self.stages
    }

    async fn run(&self, ctx: &TriggerContext) -> TriggerOutcome {
        let Some(state) = ctx.task_state.as_ref() else {
            warn!(
                stage = ?ctx.stage,
                "followup_proposer fired without TaskTriggerState; skipping"
            );
            return TriggerOutcome::Continue;
        };

        // Only propose follow-ups on the success path. Failure-derived
        // narrow spawning is the FailureExtractionTrigger's job.
        let passed = state
            .peek_validation_result()
            .map(|r| r.passed)
            .unwrap_or(false);
        if !passed {
            return TriggerOutcome::Continue;
        }

        let Some(orch) = self.orchestrator.upgrade() else {
            warn!("followup_proposer: orchestrator dropped; skipping");
            return TriggerOutcome::Continue;
        };

        // The generator inspects `task.state` to decide what to emit and
        // refuses non-terminal tasks. AfterTask fires while the in-memory
        // copy is still in `Validating`; clone and mark Accepted so the
        // generator sees the same terminal state the orchestrator is
        // about to commit.
        let mut task_snapshot = state.task.clone();
        task_snapshot.state = swell_core::TaskState::Accepted;

        let proposals = self.generator.generate_follow_ups(&task_snapshot);
        if proposals.is_empty() {
            return TriggerOutcome::Continue;
        }

        let queue = orch.proposal_queue();
        let count = proposals.len();
        for proposal in proposals {
            queue.submit(proposal);
        }
        info!(
            task_id = %state.task.id,
            proposals_enqueued = count,
            "followup_proposer enqueued follow-up proposals"
        );
        TriggerOutcome::Continue
    }
}

/// Register the `followup_proposer` factory.
///
/// Config blob is optional; recognized keys mirror
/// [`FollowUpGeneratorConfig`]:
///
/// - `min_priority: u8`
/// - `max_proposals_per_task: usize`
/// - `include_low_risk: bool`
/// - `include_medium_risk: bool`
/// - `include_high_risk: bool`
///
/// Unrecognised keys are ignored. Malformed values fall through to the
/// generator default for that field.
pub fn register_followup_proposer_factory(
    factories: &mut TriggerFactoryRegistry,
    orchestrator: Weak<Orchestrator>,
) {
    factories.register("followup_proposer", move |stages, config| {
        let leaked: &'static [Stage] = Box::leak(stages.to_vec().into_boxed_slice());
        let mut cfg = FollowUpGeneratorConfig::default();
        if let Some(v) = config
            .get("min_priority")
            .and_then(|v| v.as_u64())
            .and_then(|n| u8::try_from(n).ok())
        {
            cfg.min_priority = v;
        }
        if let Some(v) = config
            .get("max_proposals_per_task")
            .and_then(|v| v.as_u64())
        {
            cfg.max_proposals_per_task = v as usize;
        }
        if let Some(v) = config.get("include_low_risk").and_then(|v| v.as_bool()) {
            cfg.include_low_risk = v;
        }
        if let Some(v) = config.get("include_medium_risk").and_then(|v| v.as_bool()) {
            cfg.include_medium_risk = v;
        }
        if let Some(v) = config.get("include_high_risk").and_then(|v| v.as_bool()) {
            cfg.include_high_risk = v;
        }
        let trigger = FollowUpProposerTrigger::new(
            leaked,
            orchestrator.clone(),
            FollowUpGenerator::with_config(cfg),
        );
        Some(Arc::new(trigger) as Arc<dyn Trigger>)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::triggers::{TaskTriggerState, TriggerContext};
    use crate::OrchestratorBuilder;
    use std::path::PathBuf;
    use swell_core::{Plan, PlanStep, RiskLevel, StepStatus, Task};
    use swell_validation::orchestrator::{TaskExecutionMetadata, TaskValidationResult};
    use uuid::Uuid;

    fn metadata() -> TaskExecutionMetadata {
        TaskExecutionMetadata {
            completed_without_error: true,
            iteration_count: 0,
            input_tokens: 0,
            output_tokens: 0,
            duration_ms: 0,
            tool_calls_made: 0,
            max_iterations_reached: false,
        }
    }

    fn passed_validation() -> TaskValidationResult {
        TaskValidationResult {
            passed: true,
            warnings: vec!["watch this style nit".to_string()],
            ..Default::default()
        }
    }

    fn failed_validation() -> TaskValidationResult {
        TaskValidationResult {
            passed: false,
            lint_passed: false,
            tests_passed: false,
            errors: vec!["compile error".to_string()],
            ..Default::default()
        }
    }

    fn task_with_public_api_plan() -> Task {
        let mut t = Task::new("add public function".to_string());
        let plan = Plan {
            id: Uuid::new_v4(),
            task_id: t.id,
            steps: vec![PlanStep {
                id: Uuid::new_v4(),
                description: "extend public API".to_string(),
                affected_files: vec!["src/lib.rs".to_string()],
                expected_tests: Vec::new(),
                risk_level: RiskLevel::Low,
                dependencies: Vec::new(),
                status: StepStatus::Completed,
            }],
            total_estimated_tokens: 1000,
            risk_assessment: "low".to_string(),
        };
        t.plan = Some(plan);
        t
    }

    fn shared_state(task: Task, validation: Option<TaskValidationResult>) -> Arc<TaskTriggerState> {
        let state = TaskTriggerState::new(
            PathBuf::from("/tmp/swell-test"),
            Vec::new(),
            task.plan.clone(),
            metadata(),
            task,
            true,
        );
        if let Some(v) = validation {
            state.set_validation_result(v);
        }
        Arc::new(state)
    }

    /// Passing validation + a plan that touches a public API file should
    /// produce at least one proposal in the queue.
    #[tokio::test]
    async fn accepted_task_enqueues_proposals() {
        let orch = OrchestratorBuilder::new().build();
        let trigger = FollowUpProposerTrigger::after_task(Arc::downgrade(&orch));

        let task = task_with_public_api_plan();
        let task_id = task.id;
        let state = shared_state(task, Some(passed_validation()));
        let ctx = TriggerContext::for_task(Stage::AfterTask, task_id).with_task_state(state);

        assert_eq!(trigger.run(&ctx).await, TriggerOutcome::Continue);

        let queue = orch.proposal_queue();
        let pending = queue.pending();
        assert!(
            !pending.is_empty(),
            "expected the proposer to enqueue at least one follow-up"
        );
        assert!(
            pending.iter().all(|q| q.proposal.parent_task_id == task_id),
            "every proposal must reference the parent task"
        );
    }

    /// Failed validation must short-circuit: failures are
    /// `FailureExtractionTrigger`'s job, not the proposer's.
    #[tokio::test]
    async fn failed_validation_enqueues_nothing() {
        let orch = OrchestratorBuilder::new().build();
        let trigger = FollowUpProposerTrigger::after_task(Arc::downgrade(&orch));

        let task = task_with_public_api_plan();
        let task_id = task.id;
        let state = shared_state(task, Some(failed_validation()));
        let ctx = TriggerContext::for_task(Stage::AfterTask, task_id).with_task_state(state);

        let _ = trigger.run(&ctx).await;
        assert!(orch.proposal_queue().is_empty());
    }

    /// No validation result in the slot → nothing enqueued.
    #[tokio::test]
    async fn missing_validation_result_enqueues_nothing() {
        let orch = OrchestratorBuilder::new().build();
        let trigger = FollowUpProposerTrigger::after_task(Arc::downgrade(&orch));

        let task = task_with_public_api_plan();
        let task_id = task.id;
        let state = shared_state(task, None);
        let ctx = TriggerContext::for_task(Stage::AfterTask, task_id).with_task_state(state);

        let _ = trigger.run(&ctx).await;
        assert!(orch.proposal_queue().is_empty());
    }

    /// Approving a queued proposal yields a `Task` that links back to the
    /// parent so the orchestrator can pick it up after promotion. Pins the
    /// `FollowUpProposal::into_task` contract through the trigger.
    #[tokio::test]
    async fn approved_proposal_converts_to_task_with_parent_link() {
        let orch = OrchestratorBuilder::new().build();
        let trigger = FollowUpProposerTrigger::after_task(Arc::downgrade(&orch));

        let task = task_with_public_api_plan();
        let task_id = task.id;
        let state = shared_state(task, Some(passed_validation()));
        let ctx = TriggerContext::for_task(Stage::AfterTask, task_id).with_task_state(state);
        let _ = trigger.run(&ctx).await;

        let queue = orch.proposal_queue();
        let entry = queue
            .pending()
            .into_iter()
            .next()
            .expect("proposal should have been enqueued");
        let drained = queue.approve(entry.proposal.id).expect("approve");
        let derived = drained.into_task();
        assert_eq!(derived.parent, Some(task_id));
        assert!(derived.dependencies.contains(&task_id));
    }

    /// Factory parses `max_proposals_per_task` and clamps the queue size
    /// accordingly. Verifies the config-loader path.
    #[tokio::test]
    async fn factory_honors_max_proposals_per_task() {
        let orch = OrchestratorBuilder::new().build();
        let mut factories = TriggerFactoryRegistry::new();
        register_followup_proposer_factory(&mut factories, Arc::downgrade(&orch));

        let cfg: crate::trigger_config::TriggerConfig = serde_json::from_str(
            r#"{ "followup_proposer": { "stages": ["AfterTask"], "config": { "max_proposals_per_task": 1 } } }"#,
        )
        .unwrap();
        let loaded = crate::trigger_config::build_triggers(&cfg, &factories);
        assert_eq!(loaded.built.len(), 1);
        let trigger = loaded.built.into_iter().next().unwrap();

        let task = task_with_public_api_plan();
        let task_id = task.id;
        let state = shared_state(task, Some(passed_validation()));
        let ctx = TriggerContext::for_task(Stage::AfterTask, task_id).with_task_state(state);
        let _ = trigger.run(&ctx).await;

        let pending = orch.proposal_queue().pending();
        assert_eq!(
            pending.len(),
            1,
            "factory config should cap enqueued proposals at 1"
        );
    }

    /// Factory parses `min_priority` and filters low-priority proposals
    /// out. With a high enough threshold an accepted-with-warnings task
    /// emits nothing.
    #[tokio::test]
    async fn factory_honors_min_priority() {
        let orch = OrchestratorBuilder::new().build();
        let mut factories = TriggerFactoryRegistry::new();
        register_followup_proposer_factory(&mut factories, Arc::downgrade(&orch));

        let cfg: crate::trigger_config::TriggerConfig = serde_json::from_str(
            r#"{ "followup_proposer": { "stages": ["AfterTask"], "config": { "min_priority": 200 } } }"#,
        )
        .unwrap();
        let loaded = crate::trigger_config::build_triggers(&cfg, &factories);
        let trigger = loaded.built.into_iter().next().unwrap();

        // Plain accepted task with no public API touch: every default
        // opportunity has priority < 200, so the filter must drain.
        let mut task = Task::new("trivial".to_string());
        task.validation_result = Some(swell_core::ValidationResult {
            passed: true,
            lint_passed: true,
            tests_passed: true,
            security_passed: true,
            ai_review_passed: true,
            errors: Vec::new(),
            warnings: Vec::new(),
        });
        let task_id = task.id;
        let state = shared_state(task, Some(passed_validation()));
        let ctx = TriggerContext::for_task(Stage::AfterTask, task_id).with_task_state(state);
        let _ = trigger.run(&ctx).await;

        assert!(
            orch.proposal_queue().is_empty(),
            "min_priority=200 must filter everything out"
        );
    }

    /// Missing `TaskTriggerState` (caller forgot to attach it) must not
    /// panic — log + Continue.
    #[tokio::test]
    async fn missing_task_state_continues_without_panic() {
        let orch = OrchestratorBuilder::new().build();
        let trigger = FollowUpProposerTrigger::after_task(Arc::downgrade(&orch));
        let ctx = TriggerContext::for_task(Stage::AfterTask, swell_core::TaskId::new());
        assert_eq!(trigger.run(&ctx).await, TriggerOutcome::Continue);
        assert!(orch.proposal_queue().is_empty());
    }

    /// Stage gating: probe that the trigger declares only `AfterTask` by
    /// default and rejects firing at other stages via the registry.
    #[tokio::test]
    async fn after_task_default_stage_only() {
        let orch = OrchestratorBuilder::new().build();
        let trigger = FollowUpProposerTrigger::after_task(Arc::downgrade(&orch));
        assert_eq!(trigger.stages(), &[Stage::AfterTask]);
        assert_eq!(trigger.name(), "followup_proposer");
    }
}
