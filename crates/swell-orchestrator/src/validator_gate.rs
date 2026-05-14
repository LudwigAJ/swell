//! Built-in `ValidatorGateTrigger` — the F3 slice of
//! `plan/flow_integration_plan/08_validation_gates.md`.
//!
//! Re-expresses the inline `ValidationOrchestrator::validate_task_completion`
//! call that today lives inside `ExecutionController::execute_task` as an
//! `AfterTask` trigger. When installed, it consumes the per-fire
//! [`TaskTriggerState`] payload (workspace path, changed files, plan,
//! execution metadata) and writes the resulting [`TaskValidationResult`] back
//! into the same payload. `execute_task` then reads that result and drives
//! commit / skill extraction / `complete_task` exactly as before.
//!
//! Behavior contract:
//!
//! - Trigger installed (default daemon path) → validation runs through the
//!   trigger, legacy inline call is skipped.
//! - Trigger **not** installed (no `.swell/triggers.json` entry, or
//!   `enabled: false`) → `execute_task` falls back to the inline
//!   `ValidationOrchestrator` so existing behavior is preserved. This is the
//!   F3 "default-on without behavior change" contract from
//!   `plan/flow_integration_plan/10_migration_plan.md`.
//!
//! The factory function [`validator_gate_factory`] is what the daemon
//! registers in [`TriggerFactoryRegistry`] so `.swell/triggers.json` entries
//! resolve to this trigger.

use std::sync::Arc;

use async_trait::async_trait;
use swell_validation::orchestrator::{TaskCompletionInput, ValidationOrchestrator};
use tracing::{info, warn};

use crate::trigger_config::TriggerFactoryRegistry;
use crate::triggers::{Stage, Trigger, TriggerContext, TriggerOutcome};

/// AfterTask trigger that runs the configured [`ValidationOrchestrator`] and
/// writes the [`TaskValidationResult`] into the per-fire shared payload.
pub struct ValidatorGateTrigger {
    stages: &'static [Stage],
    orchestrator: Arc<ValidationOrchestrator>,
}

impl ValidatorGateTrigger {
    pub fn new(stages: &'static [Stage], orchestrator: Arc<ValidationOrchestrator>) -> Self {
        Self {
            stages,
            orchestrator,
        }
    }

    /// Convenience constructor for the default `AfterTask`-only wiring.
    pub fn after_task(orchestrator: Arc<ValidationOrchestrator>) -> Self {
        Self::new(&[Stage::AfterTask], orchestrator)
    }
}

#[async_trait]
impl Trigger for ValidatorGateTrigger {
    fn name(&self) -> &'static str {
        "validator_gate"
    }

    fn stages(&self) -> &'static [Stage] {
        self.stages
    }

    async fn run(&self, ctx: &TriggerContext) -> TriggerOutcome {
        let Some(state) = ctx.task_state.as_ref() else {
            warn!(
                stage = ?ctx.stage,
                "validator_gate fired without TaskTriggerState; skipping"
            );
            return TriggerOutcome::Continue;
        };

        let task_id = match ctx.task {
            Some(id) => id,
            None => {
                warn!("validator_gate fired without task id; skipping");
                return TriggerOutcome::Continue;
            }
        };

        let input = TaskCompletionInput {
            task_id,
            workspace_path: state.workspace_path.display().to_string(),
            changed_files: state.changed_files.clone(),
            plan: state.plan.clone(),
            execution_metadata: Some(state.execution_metadata.clone()),
        };

        match self.orchestrator.validate_task_completion(input).await {
            Ok(result) => {
                let passed = result.passed;
                state.set_validation_result(result);
                info!(
                    task_id = %task_id,
                    passed = passed,
                    "validator_gate trigger produced validation result"
                );
                TriggerOutcome::Continue
            }
            Err(e) => {
                warn!(
                    task_id = %task_id,
                    error = %e,
                    "validator_gate validation errored; surfacing as failed result"
                );
                let err_msg = format!("Validation orchestrator error: {}", e);
                let failed = swell_validation::orchestrator::TaskValidationResult {
                    passed: false,
                    lint_passed: false,
                    tests_passed: false,
                    security_passed: false,
                    ai_review_passed: false,
                    errors: vec![err_msg],
                    ..Default::default()
                };
                state.set_validation_result(failed);
                TriggerOutcome::Continue
            }
        }
    }
}

/// Factory matching the [`crate::trigger_config::TriggerFactoryFn`] signature.
/// `_config` is currently ignored; future revisions can use it to select
/// fast-gates / all-gates modes.
pub fn validator_gate_factory(
    stages: &[Stage],
    _config: &serde_json::Value,
    orchestrator: Arc<ValidationOrchestrator>,
) -> Option<Arc<dyn Trigger>> {
    // The Trigger trait requires `&'static [Stage]`. Leak the resolved stage
    // list once per registered trigger — bounded by the number of trigger
    // factories registered at daemon startup, so the leak is fine.
    let leaked: &'static [Stage] = Box::leak(stages.to_vec().into_boxed_slice());
    Some(Arc::new(ValidatorGateTrigger::new(leaked, orchestrator)))
}

/// Register the `validator_gate` factory on the given
/// [`TriggerFactoryRegistry`]. Called from `Daemon::run` before triggers are
/// resolved from `.swell/triggers.json`.
pub fn register_validator_gate_factory(
    factories: &mut TriggerFactoryRegistry,
    orchestrator: Arc<ValidationOrchestrator>,
) {
    factories.register("validator_gate", move |stages, config| {
        validator_gate_factory(stages, config, orchestrator.clone())
    });
}

/// Convenience wrapper that constructs a default [`ValidationOrchestrator`]
/// and registers the `validator_gate` factory. Lets the daemon call a single
/// function without pulling `swell-validation` into its `Cargo.toml`.
pub fn register_default_validator_gate_factory(factories: &mut TriggerFactoryRegistry) {
    register_validator_gate_factory(factories, Arc::new(ValidationOrchestrator::default()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::triggers::{TaskTriggerState, TriggerRegistry};
    use swell_core::TaskId;
    use swell_validation::orchestrator::TaskExecutionMetadata;

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

    #[tokio::test]
    async fn validator_gate_writes_result_into_task_state() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = Arc::new(TaskTriggerState::new(
            tmp.path().to_path_buf(),
            vec![],
            None,
            metadata(),
            swell_core::Task::new("validator_gate smoke".into()),
            false,
        ));

        let trigger = Arc::new(ValidatorGateTrigger::after_task(Arc::new(
            ValidationOrchestrator::default(),
        )));
        let registry = TriggerRegistry::new();
        registry.register(trigger);

        let ctx = TriggerContext::for_task(Stage::AfterTask, TaskId::new())
            .with_task_state(state.clone());
        let report = registry.fire(&ctx).await;

        assert_eq!(report.fired, vec!["validator_gate"]);
        assert!(report.all_continued());
        assert!(
            state.take_validation_result().is_some(),
            "validator_gate must populate TaskTriggerState.validation_result"
        );
    }

    #[tokio::test]
    async fn validator_gate_without_state_does_not_panic() {
        let trigger = ValidatorGateTrigger::after_task(Arc::new(ValidationOrchestrator::default()));
        let ctx = TriggerContext::for_task(Stage::AfterTask, TaskId::new());
        assert_eq!(trigger.run(&ctx).await, TriggerOutcome::Continue);
    }
}
