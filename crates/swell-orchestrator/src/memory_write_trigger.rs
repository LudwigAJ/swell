//! Built-in `MemoryWriteTrigger` — F9 of
//! `plan/flow_integration_plan/07_memory_consolidation.md`.
//!
//! Re-expresses the inline `extract_skill_candidates` call that today lives
//! inside `ExecutionController::execute_task` as an `AfterTask` trigger.
//! Behavior:
//!
//! - Reads the `TaskTriggerState` shared payload: validation result (from
//!   `validator_gate`, or whichever earlier producer ran in the same fire),
//!   task, workspace path, and the generator's `tool_calls` trace.
//! - Skips silently when validation did not pass or when the generator
//!   produced no tool calls (mirrors the legacy inline guard
//!   `generator_result.tool_calls.is_empty()`).
//! - On success calls [`TaskTriggerState::mark_memory_write_by_trigger`] so
//!   `execute_task` skips its inline fallback.
//!
//! When the trigger is **not** installed (no `.swell/triggers.json` entry,
//! or `enabled: false`), `execute_task` still runs the legacy inline skill
//! extraction. Same F3/F4 default-on-without-behavior-change contract from
//! `plan/flow_integration_plan/10_migration_plan.md`.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use swell_core::{Task, ToolCallResult};
use swell_memory::skill_extraction::{
    ExtractionConfig, SkillExtractionService, ToolCallData, TrajectoryData,
    TrajectoryStep as SkillTrajectoryStep,
};
use swell_memory::SqliteMemoryStore;
use tracing::{info, warn};

use crate::trigger_config::TriggerFactoryRegistry;
use crate::triggers::{Stage, Trigger, TriggerContext, TriggerOutcome};

/// AfterTask trigger that runs skill extraction against the per-task
/// memory store. Effectively the trigger-spine equivalent of
/// `ExecutionController::extract_skill_candidates`.
pub struct MemoryWriteTrigger {
    stages: &'static [Stage],
}

impl MemoryWriteTrigger {
    pub fn new(stages: &'static [Stage]) -> Self {
        Self { stages }
    }

    /// Convenience constructor for the default `AfterTask`-only wiring.
    pub fn after_task() -> Self {
        Self::new(&[Stage::AfterTask])
    }
}

#[async_trait]
impl Trigger for MemoryWriteTrigger {
    fn name(&self) -> &'static str {
        "memory_write"
    }

    fn stages(&self) -> &'static [Stage] {
        self.stages
    }

    async fn run(&self, ctx: &TriggerContext) -> TriggerOutcome {
        let Some(state) = ctx.task_state.as_ref() else {
            warn!(
                stage = ?ctx.stage,
                "memory_write fired without TaskTriggerState; skipping"
            );
            return TriggerOutcome::Continue;
        };

        let passed = state
            .peek_validation_result()
            .map(|r| r.passed)
            .unwrap_or(false);
        if !passed {
            return TriggerOutcome::Continue;
        }

        if state.tool_calls.is_empty() {
            // Legacy inline guard: skill extraction is a no-op without a
            // tool-call trace. Mark handled so the fallback also skips.
            state.mark_memory_write_by_trigger();
            return TriggerOutcome::Continue;
        }

        match run_skill_extraction(&state.task, &state.tool_calls, &state.workspace_path).await {
            Ok(()) => {
                state.mark_memory_write_by_trigger();
                TriggerOutcome::Continue
            }
            Err(e) => {
                // Match the legacy behavior: log and continue. Skill
                // extraction failure must not block the accepted task.
                warn!(
                    task_id = %state.task.id,
                    error = %e,
                    "memory_write trigger skill extraction failed"
                );
                // Mark handled so the inline fallback doesn't double-log
                // the same failure on the same trace.
                state.mark_memory_write_by_trigger();
                TriggerOutcome::Continue
            }
        }
    }
}

async fn run_skill_extraction(
    task: &Task,
    tool_calls: &[ToolCallResult],
    workspace_path: &Path,
) -> Result<(), swell_core::SwellError> {
    let swell_dir = workspace_path.join(".swell");
    tokio::fs::create_dir_all(&swell_dir).await.map_err(|e| {
        swell_core::SwellError::IoError(std::io::Error::new(
            e.kind(),
            format!(
                "Failed to create memory directory {}: {}",
                swell_dir.display(),
                e
            ),
        ))
    })?;

    let memory_db_path = swell_dir.join("memory.db");
    let database_url = format!("sqlite:{}?mode=rwc", memory_db_path.display());
    let store = SqliteMemoryStore::create(&database_url).await?;
    let service = SkillExtractionService::with_config(
        store,
        ExtractionConfig {
            store_path: ".swell/skills/_candidates".to_string(),
            ..ExtractionConfig::default()
        },
        workspace_path.to_path_buf(),
    );

    let plan_steps = task
        .plan
        .as_ref()
        .map(|plan| {
            plan.steps
                .iter()
                .map(|step| SkillTrajectoryStep {
                    step_id: step.id,
                    description: step.description.clone(),
                    affected_files: step.affected_files.clone(),
                    risk_level: format!("{:?}", step.risk_level),
                    status: format!("{:?}", step.status),
                })
                .collect()
        })
        .unwrap_or_default();

    let calls: Vec<ToolCallData> = tool_calls
        .iter()
        .map(|tc| ToolCallData {
            tool_name: tc.tool_name.clone(),
            arguments: tc.arguments.clone(),
            success: tc.result.is_ok(),
            timestamp: chrono::Utc::now(),
        })
        .collect();

    let files_modified: Vec<String> = tool_calls
        .iter()
        .filter_map(changed_path_from_tool_call)
        .collect();

    let tests_run = task
        .plan
        .as_ref()
        .map(|plan| {
            plan.steps
                .iter()
                .flat_map(|step| step.expected_tests.clone())
                .collect()
        })
        .unwrap_or_default();

    let trajectory = TrajectoryData {
        task_id: task.id,
        task_description: task.description.clone(),
        plan_steps,
        tool_calls: calls,
        files_modified,
        tests_run,
        validation_passed: true,
        iteration_count: tool_calls.len() as u32,
    };

    let result = service.extract_skills(trajectory).await?;
    info!(
        task_id = %task.id,
        skills_extracted = result.skills_extracted,
        patterns_found = result.patterns_found,
        "memory_write trigger completed skill extraction"
    );
    Ok(())
}

fn changed_path_from_tool_call(tc: &ToolCallResult) -> Option<String> {
    if tc.result.is_err() {
        return None;
    }
    match tc.tool_name.as_str() {
        "write_file" | "edit_file" | "multi_edit" => tc
            .arguments
            .get("path")
            .and_then(|v| v.as_str())
            .map(String::from),
        _ => None,
    }
}

/// Factory matching [`crate::trigger_config::TriggerFactoryFn`].
pub fn memory_write_factory(
    stages: &[Stage],
    _config: &serde_json::Value,
) -> Option<Arc<dyn Trigger>> {
    let leaked: &'static [Stage] = Box::leak(stages.to_vec().into_boxed_slice());
    Some(Arc::new(MemoryWriteTrigger::new(leaked)))
}

/// Register the `memory_write` factory on the given [`TriggerFactoryRegistry`].
pub fn register_memory_write_factory(factories: &mut TriggerFactoryRegistry) {
    factories.register("memory_write", memory_write_factory);
}

/// Default alias matching the `register_default_*` convention used by the
/// sibling built-in factories.
pub fn register_default_memory_write_factory(factories: &mut TriggerFactoryRegistry) {
    register_memory_write_factory(factories);
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
        let mut t = Task::new("memory_write smoke".to_string());
        t.id = TaskId::new();
        t
    }

    fn passing() -> TaskValidationResult {
        TaskValidationResult {
            passed: true,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn memory_write_skips_without_state() {
        let trigger = MemoryWriteTrigger::after_task();
        let ctx = TriggerContext::for_task(Stage::AfterTask, TaskId::new());
        assert_eq!(trigger.run(&ctx).await, TriggerOutcome::Continue);
    }

    #[tokio::test]
    async fn memory_write_skips_when_validation_not_passing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = Arc::new(TaskTriggerState::new(
            tmp.path().to_path_buf(),
            vec![],
            None,
            metadata(),
            task(),
            true,
        ));
        // No validation result written → trigger treats as "did not pass".
        let trigger = MemoryWriteTrigger::after_task();
        let ctx = TriggerContext::for_task(Stage::AfterTask, state.task.id)
            .with_task_state(Arc::clone(&state));
        let outcome = trigger.run(&ctx).await;
        assert_eq!(outcome, TriggerOutcome::Continue);
        assert!(
            !state.was_memory_write_by_trigger(),
            "trigger must not mark memory-write when validation did not pass"
        );
    }

    #[tokio::test]
    async fn memory_write_skips_when_tool_calls_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = Arc::new(
            TaskTriggerState::new(
                tmp.path().to_path_buf(),
                vec![],
                None,
                metadata(),
                task(),
                true,
            )
            .with_tool_calls(vec![]),
        );
        state.set_validation_result(passing());

        let trigger = MemoryWriteTrigger::after_task();
        let ctx = TriggerContext::for_task(Stage::AfterTask, state.task.id)
            .with_task_state(Arc::clone(&state));
        assert_eq!(trigger.run(&ctx).await, TriggerOutcome::Continue);
        assert!(
            state.was_memory_write_by_trigger(),
            "empty-tool-calls path still marks handled so the inline fallback also skips"
        );
    }

    /// Registry-path smoke: prove the trigger registers and fires through
    /// the public `TriggerRegistry` surface, paired with a counting probe
    /// to show the registry actually invokes it.
    #[tokio::test]
    async fn memory_write_fires_through_registry() {
        struct Probe {
            runs: Arc<AtomicUsize>,
        }
        #[async_trait]
        impl Trigger for Probe {
            fn name(&self) -> &'static str {
                "memory_write_probe"
            }
            fn stages(&self) -> &'static [Stage] {
                &[Stage::AfterTask]
            }
            async fn run(&self, _ctx: &TriggerContext) -> TriggerOutcome {
                self.runs.fetch_add(1, Ordering::SeqCst);
                TriggerOutcome::Continue
            }
        }

        let tmp = tempfile::TempDir::new().unwrap();
        let state = Arc::new(TaskTriggerState::new(
            tmp.path().to_path_buf(),
            vec![],
            None,
            metadata(),
            task(),
            true,
        ));

        let runs = Arc::new(AtomicUsize::new(0));
        let registry = TriggerRegistry::new();
        registry.register(Arc::new(Probe {
            runs: Arc::clone(&runs),
        }));
        registry.register(Arc::new(MemoryWriteTrigger::after_task()));

        let ctx = TriggerContext::for_task(Stage::AfterTask, state.task.id)
            .with_task_state(Arc::clone(&state));
        let report = registry.fire(&ctx).await;
        assert!(report.all_continued());
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }
}
