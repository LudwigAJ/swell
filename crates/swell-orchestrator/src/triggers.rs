//! Trigger registry — the spine that PR `02` of `plan/flow_integration_plan`
//! introduces.
//!
//! This module defines the public API surface only. It is **not yet wired
//! into `execute_task`**; the integration slice that fires triggers at
//! `BeforeTask` / `AfterTask` / milestone boundaries will follow once the
//! Goal → Milestone → Task spine from PR `01` is being driven end-to-end.
//!
//! The intent is to let later PRs (`08` validator gates, `09` git auto-commit,
//! `07` memory write) re-express their behavior as built-in `Trigger`
//! implementations against a stable API rather than reaching deeper into
//! `Orchestrator::execute_task`.

use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use swell_core::{MilestoneId, Plan, ProjectId, Task, TaskId, ToolCallResult};
use swell_validation::orchestrator::{TaskExecutionMetadata, TaskValidationResult};

/// Shared mutable payload that `AfterTask` (and future `AfterMilestone`)
/// triggers can read from and write into during a single fire cycle.
///
/// PR `02` defined `TriggerOutcome` as `Continue` / `Halt` / `Reroute` — a
/// strict control-flow channel with no data slot. The F3 slice from
/// `plan/flow_integration_plan/08_validation_gates.md` needs a
/// `ValidatorGateTrigger` to *produce* a [`TaskValidationResult`] that
/// `execute_task` then uses to drive commit / skill extraction /
/// `complete_task`. Threading the result back through `TriggerOutcome` would
/// either bloat the enum or invent a stage-specific variant; sharing it
/// through a per-fire payload keeps the registry surface clean and lets
/// successor triggers (`GitCommitTrigger`, `MemoryWriteTrigger`) consume
/// what `ValidatorGateTrigger` produced earlier in the same stage's
/// registration order.
///
/// All fields are populated by the orchestrator before firing; only
/// `validation_result` is written by triggers.
pub struct TaskTriggerState {
    pub workspace_path: PathBuf,
    pub changed_files: Vec<String>,
    pub plan: Option<Plan>,
    pub execution_metadata: TaskExecutionMetadata,
    /// The task being executed. Populated by `ExecutionController` before
    /// firing so triggers that need to build commit messages, memory
    /// payloads, etc. don't have to call back into the orchestrator.
    pub task: Task,
    /// Whether the task is running inside a dedicated worktree. Triggers
    /// that touch git (e.g. `git_commit`) must skip when this is `false`
    /// — same predicate as the legacy `worktree_allocation.is_some()` gate.
    pub worktree_allocated: bool,
    /// Output slot: a trigger (validator_gate) writes the validation result
    /// here; `execute_task` reads it after the AfterTask fire completes. A
    /// `None` here after fire means no trigger took responsibility for
    /// validation, so the legacy inline `ValidationOrchestrator` call runs
    /// as a fallback — this is what preserves the F3 "default-on without
    /// behavior change" contract when `.swell/triggers.json` is missing.
    pub validation_result: RwLock<Option<TaskValidationResult>>,
    /// Flipped by `GitCommitTrigger` once it has taken responsibility for
    /// committing the task's diff. `execute_task` reads this after the
    /// AfterTask fire to decide whether to run its legacy inline
    /// `commit_successful_task` call as a fallback. Same default-on-without
    /// -behavior-change pattern as the validation slot above.
    pub committed_by_trigger: AtomicBool,
    /// Tool calls produced by the Generator agent during the task. Consumed
    /// by `MemoryWriteTrigger` (F9, `plan/flow_integration_plan/07_memory_consolidation.md`)
    /// to re-express the inline `extract_skill_candidates` side effect that
    /// today runs inside `ExecutionController::execute_task`.
    pub tool_calls: Vec<ToolCallResult>,
    /// Flipped by `MemoryWriteTrigger` once it has run skill extraction /
    /// memory writes against the task. `execute_task` reads this after the
    /// AfterTask fire to decide whether to run its legacy inline
    /// `extract_skill_candidates` call as a fallback. Mirrors
    /// `committed_by_trigger` above.
    pub memory_write_by_trigger: AtomicBool,
}

impl std::fmt::Debug for TaskTriggerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskTriggerState")
            .field("workspace_path", &self.workspace_path)
            .field("changed_files", &self.changed_files.len())
            .field("plan", &self.plan.is_some())
            .field(
                "validation_result_present",
                &self
                    .validation_result
                    .read()
                    .map(|g| g.is_some())
                    .unwrap_or(false),
            )
            .finish()
    }
}

impl TaskTriggerState {
    pub fn new(
        workspace_path: PathBuf,
        changed_files: Vec<String>,
        plan: Option<Plan>,
        execution_metadata: TaskExecutionMetadata,
        task: Task,
        worktree_allocated: bool,
    ) -> Self {
        Self {
            workspace_path,
            changed_files,
            plan,
            execution_metadata,
            task,
            worktree_allocated,
            validation_result: RwLock::new(None),
            committed_by_trigger: AtomicBool::new(false),
            tool_calls: Vec::new(),
            memory_write_by_trigger: AtomicBool::new(false),
        }
    }

    /// Builder-style override of the generator tool-call list. Used by
    /// `ExecutionController::execute_task` to thread the Generator's tool
    /// trace through to `MemoryWriteTrigger` without breaking existing
    /// callers of `TaskTriggerState::new`.
    pub fn with_tool_calls(mut self, tool_calls: Vec<ToolCallResult>) -> Self {
        self.tool_calls = tool_calls;
        self
    }

    /// Take the validation result written by a trigger, if any.
    pub fn take_validation_result(&self) -> Option<TaskValidationResult> {
        self.validation_result
            .write()
            .ok()
            .and_then(|mut g| g.take())
    }

    /// Non-destructive read of the validation result for triggers that need
    /// to see what an earlier trigger (e.g. `validator_gate`) produced
    /// during the same fire — `take_validation_result` is reserved for
    /// `execute_task`, which consumes the slot once at the end of fire.
    pub fn peek_validation_result(&self) -> Option<TaskValidationResult> {
        self.validation_result
            .read()
            .ok()
            .and_then(|g| g.as_ref().cloned())
    }

    /// Set the validation result. Used by validator-gate triggers; later
    /// writers overwrite earlier ones in registration order.
    pub fn set_validation_result(&self, result: TaskValidationResult) {
        if let Ok(mut g) = self.validation_result.write() {
            *g = Some(result);
        }
    }

    /// Mark that a trigger has taken responsibility for committing the
    /// task's diff. `execute_task` skips its inline commit when this is set.
    pub fn mark_committed_by_trigger(&self) {
        self.committed_by_trigger.store(true, Ordering::SeqCst);
    }

    pub fn was_committed_by_trigger(&self) -> bool {
        self.committed_by_trigger.load(Ordering::SeqCst)
    }

    /// Mark that a trigger has run the memory-write / skill-extraction side
    /// effect for this task. `execute_task` skips its inline
    /// `extract_skill_candidates` call when this is set.
    pub fn mark_memory_write_by_trigger(&self) {
        self.memory_write_by_trigger.store(true, Ordering::SeqCst);
    }

    pub fn was_memory_write_by_trigger(&self) -> bool {
        self.memory_write_by_trigger.load(Ordering::SeqCst)
    }
}

/// Lifecycle edges at which the orchestrator fires registered triggers.
///
/// `BeforeTask` / `AfterTask` map onto the current `execute_task` boundaries.
/// The milestone- and project-scoped stages light up once PR `03`
/// (`MilestoneScheduler`) is driving the spine. `OnTaskFailed` and
/// `OnMilestoneBlocked` are routed to from the orchestrator's existing
/// failure paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Stage {
    BeforeTask,
    AfterTask,
    BeforeMilestone,
    AfterMilestone,
    BeforeProject,
    AfterProject,
    OnTaskFailed,
    OnMilestoneBlocked,
}

/// Per-fire context handed to every trigger. Fields that are not meaningful
/// for the firing stage are `None` — e.g. `task` is `None` during
/// `BeforeProject`.
#[derive(Debug, Clone)]
pub struct TriggerContext {
    pub stage: Stage,
    pub project: Option<ProjectId>,
    pub milestone: Option<MilestoneId>,
    pub task: Option<TaskId>,
    /// Optional shared payload for task-scoped stages (`BeforeTask` /
    /// `AfterTask`). `None` for milestone/project stages and for callers
    /// that don't need triggers to exchange data (e.g. the BeforeTask halt
    /// path). See [`TaskTriggerState`].
    pub task_state: Option<Arc<TaskTriggerState>>,
}

impl TriggerContext {
    pub fn for_task(stage: Stage, task: TaskId) -> Self {
        Self {
            stage,
            project: None,
            milestone: None,
            task: Some(task),
            task_state: None,
        }
    }

    pub fn for_milestone(stage: Stage, project: ProjectId, milestone: MilestoneId) -> Self {
        Self {
            stage,
            project: Some(project),
            milestone: Some(milestone),
            task: None,
            task_state: None,
        }
    }

    pub fn for_project(stage: Stage, project: ProjectId) -> Self {
        Self {
            stage,
            project: Some(project),
            milestone: None,
            task: None,
            task_state: None,
        }
    }

    /// Attach a shared [`TaskTriggerState`] to this context. Used by
    /// `ExecutionController` when firing `AfterTask` so the
    /// `ValidatorGateTrigger` can write its [`TaskValidationResult`] back
    /// for `execute_task` to consume.
    pub fn with_task_state(mut self, state: Arc<TaskTriggerState>) -> Self {
        self.task_state = Some(state);
        self
    }
}

/// Result of running a single trigger.
///
/// `Halt` short-circuits the remaining triggers for this stage and bubbles
/// the reason up to the orchestrator — the existing failure path stays the
/// authority on what to do (pause / fail / escalate). `Reroute` is reserved
/// for the researcher handoff in PR `04` and the milestone scheduler in
/// PR `03`; the registry just surfaces it so the caller can act on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerOutcome {
    Continue,
    Halt(String),
    Reroute(MilestoneId),
}

/// A trigger is a piece of behavior that runs at one or more lifecycle
/// stages. Implementations stay pure: they read from the context and emit
/// an outcome. Any side effects are the implementation's responsibility,
/// not the registry's.
#[async_trait]
pub trait Trigger: Send + Sync {
    fn name(&self) -> &'static str;
    fn stages(&self) -> &'static [Stage];
    async fn run(&self, ctx: &TriggerContext) -> TriggerOutcome;
}

/// Aggregate of triggers fired by the orchestrator at lifecycle edges.
///
/// Registration order is fire order. The first non-`Continue` outcome short
/// circuits remaining triggers for the same stage and is returned to the
/// caller, paired with the triggering trigger's name for diagnostics.
/// Registry of triggers fired at lifecycle edges.
///
/// Uses interior mutability so callers holding `Arc<TriggerRegistry>`
/// (e.g. `ExecutionController`, `Orchestrator`) can register triggers
/// after construction without an exclusive borrow. This is what lets the
/// daemon bootstrap install triggers post-`Orchestrator::new` and what
/// lets tests pre-register a `HaltTrigger` against the live registry the
/// daemon will fire from.
#[derive(Default)]
pub struct TriggerRegistry {
    triggers: RwLock<Vec<Arc<dyn Trigger>>>,
}

impl std::fmt::Debug for TriggerRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names = self.names();
        f.debug_struct("TriggerRegistry")
            .field("count", &names.len())
            .field("names", &names)
            .finish()
    }
}

/// Result of firing one stage's worth of triggers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FireReport {
    pub fired: Vec<&'static str>,
    pub short_circuit: Option<(&'static str, TriggerOutcome)>,
}

impl FireReport {
    pub fn all_continued(&self) -> bool {
        self.short_circuit.is_none()
    }
}

impl TriggerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a trigger to the registry. Registration order is fire order.
    pub fn register(&self, trigger: Arc<dyn Trigger>) {
        self.triggers
            .write()
            .expect("trigger registry write lock poisoned")
            .push(trigger);
    }

    pub fn len(&self) -> usize {
        self.triggers
            .read()
            .expect("trigger registry read lock poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.triggers
            .read()
            .expect("trigger registry read lock poisoned")
            .iter()
            .map(|t| t.name())
            .collect()
    }

    fn snapshot(&self) -> Vec<Arc<dyn Trigger>> {
        self.triggers
            .read()
            .expect("trigger registry read lock poisoned")
            .iter()
            .map(Arc::clone)
            .collect()
    }

    /// Run every trigger registered for `stage` in registration order,
    /// stopping at the first non-`Continue` outcome.
    ///
    /// The trigger list is snapshotted under the read lock and the lock is
    /// released before any `await`, so concurrent registration during fire
    /// will not affect the in-flight stage and will not deadlock.
    pub async fn fire(&self, ctx: &TriggerContext) -> FireReport {
        let triggers = self.snapshot();
        let mut report = FireReport {
            fired: Vec::new(),
            short_circuit: None,
        };
        for trigger in triggers {
            if !trigger.stages().contains(&ctx.stage) {
                continue;
            }
            let name = trigger.name();
            report.fired.push(name);
            match trigger.run(ctx).await {
                TriggerOutcome::Continue => continue,
                other => {
                    report.short_circuit = Some((name, other));
                    break;
                }
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingTrigger {
        name: &'static str,
        stages: &'static [Stage],
        outcome: TriggerOutcome,
        seen: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Trigger for CountingTrigger {
        fn name(&self) -> &'static str {
            self.name
        }
        fn stages(&self) -> &'static [Stage] {
            self.stages
        }
        async fn run(&self, _ctx: &TriggerContext) -> TriggerOutcome {
            self.seen.fetch_add(1, Ordering::SeqCst);
            self.outcome.clone()
        }
    }

    fn counting(
        name: &'static str,
        stages: &'static [Stage],
        outcome: TriggerOutcome,
    ) -> (Arc<CountingTrigger>, Arc<AtomicUsize>) {
        let seen = Arc::new(AtomicUsize::new(0));
        let trigger = Arc::new(CountingTrigger {
            name,
            stages,
            outcome,
            seen: seen.clone(),
        });
        (trigger, seen)
    }

    #[tokio::test]
    async fn empty_registry_fires_nothing() {
        let registry = TriggerRegistry::new();
        let ctx = TriggerContext::for_task(Stage::AfterTask, TaskId::new());
        let report = registry.fire(&ctx).await;
        assert!(report.fired.is_empty());
        assert!(report.all_continued());
    }

    #[tokio::test]
    async fn fires_only_matching_stages_in_registration_order() {
        let (t1, s1) = counting("a", &[Stage::AfterTask], TriggerOutcome::Continue);
        let (t2, s2) = counting("b", &[Stage::BeforeTask], TriggerOutcome::Continue);
        let (t3, s3) = counting("c", &[Stage::AfterTask], TriggerOutcome::Continue);

        let registry = TriggerRegistry::new();
        registry.register(t1);
        registry.register(t2);
        registry.register(t3);

        let ctx = TriggerContext::for_task(Stage::AfterTask, TaskId::new());
        let report = registry.fire(&ctx).await;

        assert_eq!(report.fired, vec!["a", "c"]);
        assert_eq!(s1.load(Ordering::SeqCst), 1);
        assert_eq!(s2.load(Ordering::SeqCst), 0);
        assert_eq!(s3.load(Ordering::SeqCst), 1);
        assert!(report.all_continued());
    }

    #[tokio::test]
    async fn halt_short_circuits_remaining_triggers() {
        let (t1, s1) = counting("a", &[Stage::AfterTask], TriggerOutcome::Continue);
        let (t2, s2) = counting(
            "halt",
            &[Stage::AfterTask],
            TriggerOutcome::Halt("nope".into()),
        );
        let (t3, s3) = counting("c", &[Stage::AfterTask], TriggerOutcome::Continue);

        let registry = TriggerRegistry::new();
        registry.register(t1);
        registry.register(t2);
        registry.register(t3);

        let ctx = TriggerContext::for_task(Stage::AfterTask, TaskId::new());
        let report = registry.fire(&ctx).await;

        assert_eq!(report.fired, vec!["a", "halt"]);
        assert_eq!(s1.load(Ordering::SeqCst), 1);
        assert_eq!(s2.load(Ordering::SeqCst), 1);
        assert_eq!(s3.load(Ordering::SeqCst), 0);
        match report.short_circuit {
            Some(("halt", TriggerOutcome::Halt(reason))) => assert_eq!(reason, "nope"),
            other => panic!("expected halt short-circuit, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn reroute_surfaces_target_milestone() {
        let target = MilestoneId::new();
        let (t1, _) = counting(
            "reroute",
            &[Stage::AfterTask],
            TriggerOutcome::Reroute(target),
        );

        let registry = TriggerRegistry::new();
        registry.register(t1);

        let ctx = TriggerContext::for_task(Stage::AfterTask, TaskId::new());
        let report = registry.fire(&ctx).await;
        match report.short_circuit {
            Some(("reroute", TriggerOutcome::Reroute(id))) => assert_eq!(id, target),
            other => panic!("expected reroute, got {other:?}"),
        }
    }
}
