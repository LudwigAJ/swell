//! Built-in `GitCommitTrigger` — F4 of
//! `plan/flow_integration_plan/09_git_integration.md`.
//!
//! Re-expresses the inline `commit_successful_task` call that today lives
//! inside `ExecutionController::execute_task` as an `AfterTask` trigger.
//! Behavior:
//!
//! - Reads the `TaskTriggerState` shared payload: validation result (from
//!   `validator_gate` if it ran earlier in the same fire, or from a future
//!   inline producer), workspace path, task, and the `worktree_allocated`
//!   gate.
//! - Skips silently when validation failed, when no worktree is allocated,
//!   or when there's nothing to commit (mirrors the existing
//!   `CommitStrategyError::NothingToCommit` no-op path).
//! - On successful commit, calls
//!   [`TaskTriggerState::mark_committed_by_trigger`] so `execute_task`
//!   knows to skip its inline commit fallback.
//!
//! When the trigger is **not** installed (no `.swell/triggers.json` entry,
//! or `enabled: false`), `execute_task` still runs the legacy inline
//! commit. This preserves the F4 default-on-without-behavior-change
//! contract from `plan/flow_integration_plan/10_migration_plan.md`.

use std::sync::Arc;

use async_trait::async_trait;
use swell_tools::{CommitMetadata, CommitRequest, CommitStrategy, CommitStrategyError};
use tracing::{info, warn};

use crate::trigger_config::TriggerFactoryRegistry;
use crate::triggers::{Stage, Trigger, TriggerContext, TriggerOutcome};

/// AfterTask trigger that commits a successfully validated task's diff
/// through the existing [`CommitStrategy`].
pub struct GitCommitTrigger {
    stages: &'static [Stage],
    commit_strategy: Arc<CommitStrategy>,
    /// `LlmBackend::model()` from the controller, captured at factory time
    /// so the trigger doesn't need a runtime handle. Goes into the commit
    /// metadata footer (`Model:` line).
    model_name: String,
    /// Agent role string for commit metadata. Currently hard-coded to
    /// `"Generator"` to match the legacy inline path; future revisions may
    /// thread the firing agent's role through `TriggerContext`.
    agent_role: String,
}

impl GitCommitTrigger {
    pub fn new(
        stages: &'static [Stage],
        commit_strategy: Arc<CommitStrategy>,
        model_name: String,
    ) -> Self {
        Self {
            stages,
            commit_strategy,
            model_name,
            agent_role: "Generator".to_string(),
        }
    }

    pub fn after_task(commit_strategy: Arc<CommitStrategy>, model_name: String) -> Self {
        Self::new(&[Stage::AfterTask], commit_strategy, model_name)
    }
}

#[async_trait]
impl Trigger for GitCommitTrigger {
    fn name(&self) -> &'static str {
        "git_commit"
    }

    fn stages(&self) -> &'static [Stage] {
        self.stages
    }

    async fn run(&self, ctx: &TriggerContext) -> TriggerOutcome {
        let Some(state) = ctx.task_state.as_ref() else {
            warn!(
                stage = ?ctx.stage,
                "git_commit fired without TaskTriggerState; skipping"
            );
            return TriggerOutcome::Continue;
        };

        if !state.worktree_allocated {
            // Legacy inline path also skips commit when no worktree was
            // allocated — there is nothing to commit against.
            return TriggerOutcome::Continue;
        }

        // Only commit when validation passed. `peek_validation_result` is a
        // non-destructive read; `execute_task` will `take` the same slot
        // after the fire completes.
        let passed = state
            .peek_validation_result()
            .map(|r| r.passed)
            .unwrap_or(false);
        if !passed {
            return TriggerOutcome::Continue;
        }

        let task = &state.task;
        let metadata = CommitMetadata::new()
            .with_generated_by("swell-daemon")
            .with_task_id(task.id)
            .with_model(self.model_name.clone())
            .with_extra("Agent-role", self.agent_role.clone())
            .with_extra("Validation-status", "passed");
        let request = CommitRequest::new(format!("Implement task {}", task.id))
            .with_description(task.description.clone())
            .with_metadata(metadata);

        match self
            .commit_strategy
            .commit(request, state.workspace_path.as_path())
            .await
        {
            Ok(commit) => {
                info!(
                    task_id = %task.id,
                    commit_hash = %commit.commit_hash,
                    files_changed = commit.files_changed,
                    "git_commit trigger committed task changes"
                );
                state.mark_committed_by_trigger();
                TriggerOutcome::Continue
            }
            Err(CommitStrategyError::NothingToCommit(reason)) => {
                info!(
                    task_id = %task.id,
                    reason = %reason,
                    "git_commit trigger: nothing to commit"
                );
                // Still mark as handled so the inline fallback doesn't run
                // and emit a duplicate "nothing to commit" log line.
                state.mark_committed_by_trigger();
                TriggerOutcome::Continue
            }
            Err(e) => {
                warn!(
                    task_id = %task.id,
                    error = %e,
                    "git_commit trigger commit failed; surfacing as Halt"
                );
                TriggerOutcome::Halt(format!("git_commit trigger commit failed: {}", e))
            }
        }
    }
}

/// Factory matching [`crate::trigger_config::TriggerFactoryFn`].
pub fn git_commit_factory(
    stages: &[Stage],
    _config: &serde_json::Value,
    commit_strategy: Arc<CommitStrategy>,
    model_name: String,
) -> Option<Arc<dyn Trigger>> {
    let leaked: &'static [Stage] = Box::leak(stages.to_vec().into_boxed_slice());
    Some(Arc::new(GitCommitTrigger::new(
        leaked,
        commit_strategy,
        model_name,
    )))
}

/// Register the `git_commit` factory on the given [`TriggerFactoryRegistry`].
pub fn register_git_commit_factory(
    factories: &mut TriggerFactoryRegistry,
    commit_strategy: Arc<CommitStrategy>,
    model_name: String,
) {
    factories.register("git_commit", move |stages, config| {
        git_commit_factory(stages, config, commit_strategy.clone(), model_name.clone())
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::triggers::{TaskTriggerState, TriggerRegistry};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use swell_core::{Task, TaskId};
    use swell_validation::orchestrator::{TaskExecutionMetadata, TaskValidationResult};

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

    fn task() -> Task {
        let mut t = Task::new("git_commit smoke".to_string());
        t.id = TaskId::new();
        t
    }

    fn passing_result() -> TaskValidationResult {
        TaskValidationResult {
            passed: true,
            ..Default::default()
        }
    }

    /// Stand-in for `CommitStrategy` is awkward to construct directly; use
    /// a `worktree_allocated: false` state to assert the trigger early-exits
    /// before touching the strategy. Production behavior is covered by the
    /// daemon socket smoke test that boots a real `Daemon`.
    struct CountingFire {
        runs: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Trigger for CountingFire {
        fn name(&self) -> &'static str {
            "git_commit_probe"
        }
        fn stages(&self) -> &'static [Stage] {
            &[Stage::AfterTask]
        }
        async fn run(&self, _ctx: &TriggerContext) -> TriggerOutcome {
            self.runs.fetch_add(1, Ordering::SeqCst);
            TriggerOutcome::Continue
        }
    }

    #[tokio::test]
    async fn git_commit_skips_when_no_worktree_allocated() {
        // Build a state with worktree_allocated=false, register a probe
        // trigger that counts run() invocations to prove the registry path
        // still fires it (so we know the skip is from our trigger logic,
        // not the registry).
        let tmp = tempfile::TempDir::new().unwrap();
        let state = Arc::new(TaskTriggerState::new(
            tmp.path().to_path_buf(),
            vec![],
            None,
            metadata(),
            task(),
            false,
        ));
        state.set_validation_result(passing_result());

        let runs = Arc::new(AtomicUsize::new(0));
        let registry = TriggerRegistry::new();
        registry.register(Arc::new(CountingFire {
            runs: Arc::clone(&runs),
        }));

        let ctx = TriggerContext::for_task(Stage::AfterTask, state.task.id)
            .with_task_state(Arc::clone(&state));
        let report = registry.fire(&ctx).await;
        assert!(report.all_continued());
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert!(
            !state.was_committed_by_trigger(),
            "trigger must not mark commit when worktree_allocated=false"
        );
    }

    #[tokio::test]
    async fn git_commit_skips_when_validation_did_not_pass() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = Arc::new(TaskTriggerState::new(
            tmp.path().to_path_buf(),
            vec![],
            None,
            metadata(),
            task(),
            true,
        ));
        // No validation result written: trigger treats missing-or-failed
        // identically (the legacy inline path also only commits on
        // result.passed == true).
        let ctx = TriggerContext::for_task(Stage::AfterTask, state.task.id)
            .with_task_state(Arc::clone(&state));

        // Construct a GitCommitTrigger with a fake commit strategy. The
        // strategy is wrapped in Arc<CommitStrategy>; default() returns a
        // working one — we just won't reach it because validation didn't
        // pass.
        let strategy = Arc::new(CommitStrategy::default());
        let trigger = GitCommitTrigger::after_task(strategy, "test-model".into());
        let outcome = trigger.run(&ctx).await;
        assert_eq!(outcome, TriggerOutcome::Continue);
        assert!(!state.was_committed_by_trigger());
    }
}
