//! F5 scaffold of [`plan/flow_integration_plan/03_worker_pool_fanout.md`].
//!
//! Drives the Goal → Milestone → Task spine: walks a project's milestones in
//! dependency order, calls [`ExecutionController::execute_task`] for each task
//! in the milestone, and fires the milestone-scoped lifecycle stages
//! ([`Stage::BeforeMilestone`], [`Stage::AfterMilestone`],
//! [`Stage::OnMilestoneBlocked`]) on the live [`TriggerRegistry`].
//!
//! Scope of this slice:
//!
//! - Tasks within a milestone are executed sequentially by default;
//!   `milestone.parallel_tasks=true` opts into concurrent dispatch via
//!   [`ExecutionController::execute_batch`] (bounded by
//!   `max_concurrent`). Sequential preserves early-exit on first
//!   failure; parallel lets all in-flight siblings finish before
//!   reporting failure.
//! - Milestones are executed in DAG order via [`Milestone::ready_given`].
//!   Cycles or unsatisfiable dependencies surface as
//!   [`MilestoneSchedulerError::Stalled`] rather than infinite loops.
//! - `BeforeMilestone` halt → milestone marked
//!   [`MilestoneStatus::Blocked`], `OnMilestoneBlocked` fires, scheduler
//!   moves on to the next ready milestone. Dependents stay `Pending` —
//!   the next `run_project` call observes them as unsatisfiable and reports
//!   `Stalled`.
//! - Task failure / rejection → milestone marked
//!   [`MilestoneStatus::Blocked`], `OnMilestoneBlocked` fires, scheduler
//!   stops walking that branch. Sibling milestones whose deps are independent
//!   are still attempted.
//! - `AfterMilestone` halt → same blocked path as task failure.
//! - `Reroute` outcomes are consumed: `BeforeMilestone` Reroute marks
//!   the source `Blocked` (work not performed) and queues the target
//!   as the next walked milestone, overriding DAG order;
//!   `AfterMilestone` Reroute leaves the source `Done` and only
//!   redirects the next walk; `AfterTask` Reroute hints are surfaced
//!   from [`ExecutionController::take_reroute_hint`] after the
//!   milestone reaches `Done` and override `forced_next` last-writer-
//!   wins. When the target is missing / already handled / not yet
//!   ready, the scheduler logs a warning and falls back to DAG order.
//!
//! The scheduler is a stateless service struct — callers (daemon CLI
//! command, tests) construct it from an [`Orchestrator`] + execution
//! controller + trigger registry and invoke [`MilestoneScheduler::run_project`].

use std::collections::HashSet;
use std::sync::Arc;

use swell_core::{MilestoneId, MilestoneStatus, ProjectId, SwellError, TaskState};
use tracing::{info, warn};

use crate::execution::ExecutionController;
use crate::triggers::{Stage, TriggerContext, TriggerOutcome, TriggerRegistry};
use crate::Orchestrator;

/// Outcome of a single milestone within a scheduler run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MilestoneOutcome {
    /// All tasks accepted; `AfterMilestone` did not halt.
    Done,
    /// `BeforeMilestone` halted before any task ran.
    BlockedBeforeStart { reason: String },
    /// A task in the milestone failed or was rejected.
    BlockedByTaskFailure { failing_task: swell_core::TaskId },
    /// `AfterMilestone` returned `Halt`.
    BlockedAfterTasks { reason: String },
}

impl MilestoneOutcome {
    pub fn is_done(&self) -> bool {
        matches!(self, MilestoneOutcome::Done)
    }
}

/// Aggregate outcome of a scheduler run over a project.
#[derive(Debug, Clone)]
pub struct MilestoneSchedulerReport {
    pub project: ProjectId,
    /// Outcome per attempted milestone in the order they were walked.
    pub attempted: Vec<(MilestoneId, MilestoneOutcome)>,
    /// Milestones that never became Ready because dependencies were
    /// unsatisfiable (cycle or upstream Blocked/Failed).
    pub stalled: Vec<MilestoneId>,
    /// `(from, to)` reroute hints surfaced during the walk. A `from`
    /// entry means a trigger registered against that milestone
    /// (`BeforeMilestone` / `AfterMilestone`) or one of its tasks
    /// (`AfterTask`) returned [`TriggerOutcome::Reroute`] pointing at
    /// `to`. The scheduler honors these as a `forced_next` override of
    /// DAG order, falling back gracefully when the target is missing,
    /// already handled, or not yet ready. See
    /// `plan/flow_integration_plan/03_worker_pool_fanout.md`.
    pub reroutes: Vec<(MilestoneId, MilestoneId)>,
}

impl MilestoneSchedulerReport {
    pub fn all_done(&self) -> bool {
        self.stalled.is_empty() && self.attempted.iter().all(|(_, o)| o.is_done())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MilestoneSchedulerError {
    #[error("orchestrator dropped before scheduler ran")]
    OrchestratorDropped,

    #[error("project {0} not found")]
    ProjectNotFound(ProjectId),

    #[error(transparent)]
    Swell(#[from] SwellError),
}

/// Drives milestones for a single project.
pub struct MilestoneScheduler {
    orchestrator: std::sync::Weak<Orchestrator>,
    controller: Arc<ExecutionController>,
    triggers: Arc<TriggerRegistry>,
}

impl MilestoneScheduler {
    pub fn new(
        orchestrator: std::sync::Weak<Orchestrator>,
        controller: Arc<ExecutionController>,
        triggers: Arc<TriggerRegistry>,
    ) -> Self {
        Self {
            orchestrator,
            controller,
            triggers,
        }
    }

    /// Convenience constructor that reuses the orchestrator's own
    /// `ExecutionController` and `TriggerRegistry`.
    pub fn from_orchestrator(orchestrator: &Arc<Orchestrator>) -> Self {
        let controller = orchestrator.execution_controller();
        let triggers = controller.trigger_registry();
        Self::new(Arc::downgrade(orchestrator), controller, triggers)
    }

    fn orchestrator(&self) -> Result<Arc<Orchestrator>, MilestoneSchedulerError> {
        self.orchestrator
            .upgrade()
            .ok_or(MilestoneSchedulerError::OrchestratorDropped)
    }

    /// Run every milestone in `project_id` in dependency order. Returns a
    /// per-milestone report. Always returns; a single milestone's halt or
    /// failure does not abort the scheduler — that is what
    /// [`MilestoneSchedulerReport::all_done`] is for.
    pub async fn run_project(
        &self,
        project_id: ProjectId,
    ) -> Result<MilestoneSchedulerReport, MilestoneSchedulerError> {
        let orchestrator = self.orchestrator()?;
        let mut report = MilestoneSchedulerReport {
            project: project_id,
            attempted: Vec::new(),
            stalled: Vec::new(),
            reroutes: Vec::new(),
        };

        // Per-run view of milestone statuses. We refresh from the
        // state-machine each iteration so `set_milestone_status` writes
        // from prior iterations are honored when computing the next ready
        // set.
        let mut handled: HashSet<MilestoneId> = HashSet::new();
        // Reroute target queued for the next iteration. Set when a
        // `TriggerOutcome::Reroute` surfaces from `BeforeMilestone`,
        // `AfterMilestone`, or any `AfterTask` fire under the current
        // milestone. Honored once, then cleared; falls back to DAG order
        // if the target can't be selected (missing / already handled /
        // not yet ready).
        let mut forced_next: Option<MilestoneId> = None;

        loop {
            let milestones = orchestrator
                .get_milestones_for_project(project_id)
                .await
                .map_err(|e| match e {
                    SwellError::InvalidStateTransition(_) => {
                        MilestoneSchedulerError::ProjectNotFound(project_id)
                    }
                    other => MilestoneSchedulerError::Swell(other),
                })?;

            let next = if let Some(target) = forced_next.take() {
                let forced = milestones.iter().find(|m| {
                    m.id == target
                        && !handled.contains(&m.id)
                        && matches!(m.status, MilestoneStatus::Pending | MilestoneStatus::Ready)
                });
                if forced.is_some() {
                    forced
                } else {
                    warn!(
                        project_id = %project_id,
                        target = %target,
                        "Reroute target unavailable (missing / already handled / wrong status); falling back to DAG order"
                    );
                    milestones.iter().find(|m| {
                        !handled.contains(&m.id)
                            && matches!(m.status, MilestoneStatus::Pending | MilestoneStatus::Ready)
                            && m.ready_given(milestones.iter())
                    })
                }
            } else {
                milestones.iter().find(|m| {
                    !handled.contains(&m.id)
                        && matches!(m.status, MilestoneStatus::Pending | MilestoneStatus::Ready)
                        && m.ready_given(milestones.iter())
                })
            };

            let Some(milestone) = next else {
                // Nothing more ready. Stalled = unhandled Pending milestones
                // whose dependencies aren't all Done (cycle, upstream blocked,
                // or upstream failed).
                for m in &milestones {
                    if !handled.contains(&m.id)
                        && matches!(m.status, MilestoneStatus::Pending | MilestoneStatus::Ready)
                    {
                        report.stalled.push(m.id);
                    }
                }
                break;
            };

            let milestone_id = milestone.id;
            let milestone_clone = milestone.clone();
            drop(milestones);

            handled.insert(milestone_id);

            // BeforeMilestone — halt blocks the milestone before tasks run.
            let before_ctx =
                TriggerContext::for_milestone(Stage::BeforeMilestone, project_id, milestone_id);
            let before = self.triggers.fire(&before_ctx).await;
            if let Some((name, TriggerOutcome::Halt(reason))) = &before.short_circuit {
                warn!(
                    project_id = %project_id,
                    milestone_id = %milestone_id,
                    trigger = %name,
                    reason = %reason,
                    "BeforeMilestone trigger halted milestone"
                );
                let _ = orchestrator
                    .set_milestone_status(milestone_id, MilestoneStatus::Blocked)
                    .await;
                if let Some(target) = self
                    .fire_on_blocked(project_id, milestone_id, reason.clone())
                    .await
                {
                    report.reroutes.push((milestone_id, target));
                    forced_next = Some(target);
                }
                report.attempted.push((
                    milestone_id,
                    MilestoneOutcome::BlockedBeforeStart {
                        reason: reason.clone(),
                    },
                ));
                continue;
            }
            if let Some((name, TriggerOutcome::Reroute(target))) = before.short_circuit {
                info!(
                    project_id = %project_id,
                    milestone_id = %milestone_id,
                    trigger = %name,
                    target = %target,
                    "BeforeMilestone reroute — milestone work not performed; jumping to target"
                );
                // Reroute *before* tasks ran means this milestone's work
                // was not performed — mark `Blocked`, like a halt, but
                // enqueue the target as the next walked milestone.
                let _ = orchestrator
                    .set_milestone_status(milestone_id, MilestoneStatus::Blocked)
                    .await;
                // The trigger already requested `target`; an
                // OnMilestoneBlocked reroute can override it
                // (last-writer-wins, matching the AfterTask convention).
                let recovery = self
                    .fire_on_blocked(project_id, milestone_id, format!("rerouted to {target}"))
                    .await;
                report.reroutes.push((milestone_id, target));
                forced_next = Some(target);
                if let Some(recovery_target) = recovery {
                    report.reroutes.push((milestone_id, recovery_target));
                    forced_next = Some(recovery_target);
                }
                report.attempted.push((
                    milestone_id,
                    MilestoneOutcome::BlockedBeforeStart {
                        reason: format!("rerouted to {target}"),
                    },
                ));
                continue;
            }

            orchestrator
                .set_milestone_status(milestone_id, MilestoneStatus::Executing)
                .await?;

            // Sequential by default; concurrent fan-out (bounded by
            // `ExecutionController::max_concurrent` via `execute_batch`'s
            // `buffer_unordered`) when `milestone.parallel_tasks` is set.
            // Sequential preserves early-exit on first failure; parallel
            // lets all in-flight siblings finish before reporting failure,
            // since they're already running.
            let task_outcomes: Vec<(swell_core::TaskId, Result<_, SwellError>)> =
                if milestone_clone.parallel_tasks {
                    let ids = milestone_clone.tasks.clone();
                    let results = self.controller.execute_batch(ids.clone()).await;
                    ids.into_iter().zip(results).collect()
                } else {
                    let mut acc = Vec::with_capacity(milestone_clone.tasks.len());
                    for task_id in &milestone_clone.tasks {
                        let res = self.controller.execute_task(*task_id).await;
                        let failed = match &res {
                            Ok(validation) => !validation.passed,
                            Err(_) => true,
                        };
                        acc.push((*task_id, res));
                        if failed {
                            break;
                        }
                    }
                    acc
                };

            let mut failure: Option<swell_core::TaskId> = None;
            for (task_id, res) in &task_outcomes {
                match res {
                    Ok(validation) if validation.passed => {}
                    Ok(_) => {
                        failure = Some(*task_id);
                        break;
                    }
                    Err(e) => {
                        warn!(
                            project_id = %project_id,
                            milestone_id = %milestone_id,
                            task_id = %task_id,
                            error = %e,
                            "MilestoneScheduler: task execution returned error"
                        );
                        failure = Some(*task_id);
                        break;
                    }
                }
            }

            if let Some(failing_task) = failure {
                let reason = format!("task {failing_task} failed/rejected");
                let _ = orchestrator
                    .set_milestone_status(milestone_id, MilestoneStatus::Blocked)
                    .await;
                // Drain an OnTaskFailed reroute hint emitted by the
                // failing task's `fire_on_task_failed` call inside
                // `execute_task`. OnMilestoneBlocked Reroute (below)
                // can override last-writer-wins.
                if let Some(target) = self.controller.take_reroute_hint(failing_task) {
                    info!(
                        project_id = %project_id,
                        milestone_id = %milestone_id,
                        task_id = %failing_task,
                        target = %target,
                        "OnTaskFailed reroute — jumping scheduler to target"
                    );
                    report.reroutes.push((milestone_id, target));
                    forced_next = Some(target);
                }
                if let Some(target) = self.fire_on_blocked(project_id, milestone_id, reason).await {
                    report.reroutes.push((milestone_id, target));
                    forced_next = Some(target);
                }
                report.attempted.push((
                    milestone_id,
                    MilestoneOutcome::BlockedByTaskFailure { failing_task },
                ));
                continue;
            }

            // Cross-check task states; a task might have ended Rejected
            // even on `Ok(validation_passed)` in some legacy paths.
            let still_failing = self
                .find_failing_task(&milestone_clone.tasks, &orchestrator)
                .await;
            if let Some(failing_task) = still_failing {
                let reason = format!("task {failing_task} ended non-Accepted");
                let _ = orchestrator
                    .set_milestone_status(milestone_id, MilestoneStatus::Blocked)
                    .await;
                if let Some(target) = self.controller.take_reroute_hint(failing_task) {
                    info!(
                        project_id = %project_id,
                        milestone_id = %milestone_id,
                        task_id = %failing_task,
                        target = %target,
                        "OnTaskFailed reroute — jumping scheduler to target"
                    );
                    report.reroutes.push((milestone_id, target));
                    forced_next = Some(target);
                }
                if let Some(target) = self.fire_on_blocked(project_id, milestone_id, reason).await {
                    report.reroutes.push((milestone_id, target));
                    forced_next = Some(target);
                }
                report.attempted.push((
                    milestone_id,
                    MilestoneOutcome::BlockedByTaskFailure { failing_task },
                ));
                continue;
            }

            // AfterMilestone — halt blocks the milestone after tasks ran.
            let after_ctx =
                TriggerContext::for_milestone(Stage::AfterMilestone, project_id, milestone_id);
            let after = self.triggers.fire(&after_ctx).await;
            if let Some((name, TriggerOutcome::Halt(reason))) = &after.short_circuit {
                warn!(
                    project_id = %project_id,
                    milestone_id = %milestone_id,
                    trigger = %name,
                    reason = %reason,
                    "AfterMilestone trigger halted milestone"
                );
                let _ = orchestrator
                    .set_milestone_status(milestone_id, MilestoneStatus::Blocked)
                    .await;
                if let Some(target) = self
                    .fire_on_blocked(project_id, milestone_id, reason.clone())
                    .await
                {
                    report.reroutes.push((milestone_id, target));
                    forced_next = Some(target);
                }
                report.attempted.push((
                    milestone_id,
                    MilestoneOutcome::BlockedAfterTasks {
                        reason: reason.clone(),
                    },
                ));
                continue;
            }
            if let Some((name, TriggerOutcome::Reroute(target))) = after.short_circuit {
                info!(
                    project_id = %project_id,
                    milestone_id = %milestone_id,
                    trigger = %name,
                    target = %target,
                    "AfterMilestone reroute — milestone work done; jumping to target"
                );
                // AfterMilestone fired after tasks succeeded, so the
                // milestone *is* done; reroute only redirects the next walk.
                report.reroutes.push((milestone_id, target));
                forced_next = Some(target);
            }

            orchestrator
                .set_milestone_status(milestone_id, MilestoneStatus::Done)
                .await?;
            // AfterTask reroute hints from any task in this milestone
            // override `forced_next` (last writer wins). We only consume
            // them when the milestone itself reached `Done`; on a
            // halt / failure path the hints stay in the controller's
            // side-channel until the next run picks them up — or are
            // overwritten if the same task re-executes.
            for (task_id, _) in &task_outcomes {
                if let Some(target) = self.controller.take_reroute_hint(*task_id) {
                    info!(
                        project_id = %project_id,
                        milestone_id = %milestone_id,
                        task_id = %task_id,
                        target = %target,
                        "AfterTask reroute — jumping to target after milestone Done"
                    );
                    report.reroutes.push((milestone_id, target));
                    forced_next = Some(target);
                }
            }
            report
                .attempted
                .push((milestone_id, MilestoneOutcome::Done));
            info!(
                project_id = %project_id,
                milestone_id = %milestone_id,
                "MilestoneScheduler completed milestone"
            );
        }

        Ok(report)
    }

    async fn find_failing_task(
        &self,
        task_ids: &[swell_core::TaskId],
        orchestrator: &Arc<Orchestrator>,
    ) -> Option<swell_core::TaskId> {
        for tid in task_ids {
            if let Ok(task) = orchestrator.get_task(*tid).await {
                if matches!(task.state, TaskState::Failed | TaskState::Rejected) {
                    return Some(*tid);
                }
            }
        }
        None
    }

    async fn fire_on_blocked(
        &self,
        project_id: ProjectId,
        milestone_id: MilestoneId,
        reason: String,
    ) -> Option<MilestoneId> {
        let ctx =
            TriggerContext::for_milestone(Stage::OnMilestoneBlocked, project_id, milestone_id);
        let report = self.triggers.fire(&ctx).await;
        match report.short_circuit {
            Some((name, TriggerOutcome::Halt(downstream))) => {
                warn!(
                    project_id = %project_id,
                    milestone_id = %milestone_id,
                    trigger = %name,
                    reason = %reason,
                    downstream = %downstream,
                    "OnMilestoneBlocked trigger returned Halt (observational — milestone already Blocked)"
                );
                None
            }
            Some((name, TriggerOutcome::Reroute(target))) => {
                info!(
                    project_id = %project_id,
                    milestone_id = %milestone_id,
                    trigger = %name,
                    target = %target,
                    reason = %reason,
                    "OnMilestoneBlocked reroute — jumping scheduler to target"
                );
                Some(target)
            }
            _ => {
                info!(
                    project_id = %project_id,
                    milestone_id = %milestone_id,
                    reason = %reason,
                    "Milestone blocked"
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::triggers::{Stage, Trigger, TriggerContext, TriggerOutcome, TriggerRegistry};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use swell_core::{Goal, MilestoneStatus};

    /// Smoke registry that records every fire by stage.
    #[derive(Default)]
    struct Recorder {
        stages: Mutex<Vec<Stage>>,
    }

    struct RecorderTrigger {
        rec: Arc<Recorder>,
        outcome_on_stage: Option<(Stage, TriggerOutcome)>,
    }

    #[async_trait]
    impl Trigger for RecorderTrigger {
        fn name(&self) -> &'static str {
            "recorder"
        }
        fn stages(&self) -> &'static [Stage] {
            &[
                Stage::BeforeMilestone,
                Stage::AfterMilestone,
                Stage::OnMilestoneBlocked,
            ]
        }
        async fn run(&self, ctx: &TriggerContext) -> TriggerOutcome {
            self.rec.stages.lock().unwrap().push(ctx.stage);
            if let Some((stage, outcome)) = &self.outcome_on_stage {
                if *stage == ctx.stage {
                    return outcome.clone();
                }
            }
            TriggerOutcome::Continue
        }
    }

    fn build_orchestrator() -> Arc<Orchestrator> {
        crate::OrchestratorBuilder::new().build()
    }

    async fn project_with_empty_milestone(orch: &Arc<Orchestrator>) -> (ProjectId, MilestoneId) {
        let project = orch
            .create_project(Goal::new("scheduler smoke", swell_core::TaskId::new()))
            .await;
        let milestone = orch
            .create_milestone(project.id, "m1".to_string())
            .await
            .unwrap();
        (project.id, milestone.id)
    }

    #[tokio::test]
    async fn empty_milestone_runs_to_done_and_fires_before_after() {
        let orch = build_orchestrator();
        let (project_id, milestone_id) = project_with_empty_milestone(&orch).await;

        let registry = orch.execution_controller().trigger_registry();
        let rec = Arc::new(Recorder::default());
        registry.register(Arc::new(RecorderTrigger {
            rec: Arc::clone(&rec),
            outcome_on_stage: None,
        }));

        let scheduler = MilestoneScheduler::from_orchestrator(&orch);
        let report = scheduler.run_project(project_id).await.unwrap();

        assert!(report.all_done(), "report: {report:?}");
        assert_eq!(report.attempted.len(), 1);
        assert_eq!(report.attempted[0].0, milestone_id);
        assert_eq!(report.attempted[0].1, MilestoneOutcome::Done);

        let stages = rec.stages.lock().unwrap().clone();
        assert_eq!(
            stages,
            vec![Stage::BeforeMilestone, Stage::AfterMilestone],
            "BeforeMilestone fires before tasks, AfterMilestone fires once after"
        );

        let m = orch.get_milestone(milestone_id).await.unwrap();
        assert_eq!(m.status, MilestoneStatus::Done);
    }

    #[tokio::test]
    async fn before_milestone_halt_blocks_and_routes_on_blocked() {
        let orch = build_orchestrator();
        let (project_id, milestone_id) = project_with_empty_milestone(&orch).await;

        let registry = orch.execution_controller().trigger_registry();
        let rec = Arc::new(Recorder::default());
        registry.register(Arc::new(RecorderTrigger {
            rec: Arc::clone(&rec),
            outcome_on_stage: Some((
                Stage::BeforeMilestone,
                TriggerOutcome::Halt("policy denied".into()),
            )),
        }));

        let scheduler = MilestoneScheduler::from_orchestrator(&orch);
        let report = scheduler.run_project(project_id).await.unwrap();
        assert_eq!(report.attempted.len(), 1);
        match &report.attempted[0].1 {
            MilestoneOutcome::BlockedBeforeStart { reason } => {
                assert!(reason.contains("policy denied"))
            }
            other => panic!("expected BlockedBeforeStart, got {other:?}"),
        }

        let stages = rec.stages.lock().unwrap().clone();
        assert!(stages.contains(&Stage::BeforeMilestone));
        assert!(stages.contains(&Stage::OnMilestoneBlocked));
        assert!(
            !stages.contains(&Stage::AfterMilestone),
            "AfterMilestone must not fire when BeforeMilestone halted"
        );

        let m = orch.get_milestone(milestone_id).await.unwrap();
        assert_eq!(m.status, MilestoneStatus::Blocked);
    }

    #[tokio::test]
    async fn after_milestone_halt_blocks_and_routes_on_blocked() {
        let orch = build_orchestrator();
        let (project_id, milestone_id) = project_with_empty_milestone(&orch).await;

        let registry = orch.execution_controller().trigger_registry();
        let rec = Arc::new(Recorder::default());
        registry.register(Arc::new(RecorderTrigger {
            rec: Arc::clone(&rec),
            outcome_on_stage: Some((
                Stage::AfterMilestone,
                TriggerOutcome::Halt("post-check failed".into()),
            )),
        }));

        let scheduler = MilestoneScheduler::from_orchestrator(&orch);
        let report = scheduler.run_project(project_id).await.unwrap();
        match &report.attempted[0].1 {
            MilestoneOutcome::BlockedAfterTasks { reason } => {
                assert!(reason.contains("post-check failed"))
            }
            other => panic!("expected BlockedAfterTasks, got {other:?}"),
        }
        let m = orch.get_milestone(milestone_id).await.unwrap();
        assert_eq!(m.status, MilestoneStatus::Blocked);
    }

    /// Two milestones, second depends on first. First halted before-start.
    /// Second must surface as stalled.
    #[tokio::test]
    async fn dependent_milestone_stalls_when_upstream_blocked() {
        let orch = build_orchestrator();
        let project = orch
            .create_project(Goal::new("dep smoke", swell_core::TaskId::new()))
            .await;
        let m1 = orch
            .create_milestone(project.id, "m1".to_string())
            .await
            .unwrap();
        let m2 = orch
            .create_milestone(project.id, "m2".to_string())
            .await
            .unwrap();
        // Wire m2 depends on m1 by re-creating m2 with a dependency. Direct
        // field mutation on the snapshot doesn't persist; the state machine
        // is the authority. There's no `add_dependency` API yet, so reach
        // through the state machine directly for the test.
        {
            let sm = orch.state_machine();
            let sm = sm.read().await;
            let _ = sm.with_milestone_mut(m2.id, |m| {
                m.depends_on.push(m1.id);
                Ok(())
            });
        }

        let registry = orch.execution_controller().trigger_registry();
        let rec = Arc::new(Recorder::default());
        registry.register(Arc::new(RecorderTrigger {
            rec: Arc::clone(&rec),
            outcome_on_stage: Some((
                Stage::BeforeMilestone,
                TriggerOutcome::Halt("block m1".into()),
            )),
        }));

        let scheduler = MilestoneScheduler::from_orchestrator(&orch);
        let report = scheduler.run_project(project.id).await.unwrap();

        assert_eq!(report.attempted.len(), 1, "only m1 should be attempted");
        assert_eq!(report.attempted[0].0, m1.id);
        assert_eq!(
            report.stalled,
            vec![m2.id],
            "m2 must stall since m1 is Blocked"
        );
    }

    /// Empty milestone with `parallel_tasks=true` still reaches Done.
    /// Proves the parallel dispatch branch is taken without affecting the
    /// terminal milestone status. See
    /// `plan/flow_integration_plan/03_worker_pool_fanout.md`.
    #[tokio::test]
    async fn parallel_empty_milestone_runs_to_done() {
        let orch = build_orchestrator();
        let (project_id, milestone_id) = project_with_empty_milestone(&orch).await;

        {
            let sm = orch.state_machine();
            let sm = sm.read().await;
            let _ = sm.with_milestone_mut(milestone_id, |m| {
                m.parallel_tasks = true;
                Ok(())
            });
        }

        let scheduler = MilestoneScheduler::from_orchestrator(&orch);
        let report = scheduler.run_project(project_id).await.unwrap();
        assert!(report.all_done(), "report: {report:?}");
        let m = orch.get_milestone(milestone_id).await.unwrap();
        assert_eq!(m.status, MilestoneStatus::Done);
    }

    /// `BeforeMilestone` Reroute jumps the walk to the target milestone
    /// even when DAG order would pick the source's siblings first. The
    /// source milestone is `Blocked` because its work was not performed.
    #[tokio::test]
    async fn before_milestone_reroute_jumps_to_target() {
        let orch = build_orchestrator();
        let project = orch
            .create_project(Goal::new("reroute before", swell_core::TaskId::new()))
            .await;
        let m_source = orch
            .create_milestone(project.id, "source".to_string())
            .await
            .unwrap();
        let m_target = orch
            .create_milestone(project.id, "target".to_string())
            .await
            .unwrap();

        // Reroute m_source → m_target on BeforeMilestone, but only fire
        // for the source milestone — once we jump, the target runs to
        // Done normally.
        struct OneShotReroute {
            source: MilestoneId,
            target: MilestoneId,
        }
        #[async_trait]
        impl Trigger for OneShotReroute {
            fn name(&self) -> &'static str {
                "reroute_before"
            }
            fn stages(&self) -> &'static [Stage] {
                &[Stage::BeforeMilestone]
            }
            async fn run(&self, ctx: &TriggerContext) -> TriggerOutcome {
                if ctx.milestone == Some(self.source) {
                    TriggerOutcome::Reroute(self.target)
                } else {
                    TriggerOutcome::Continue
                }
            }
        }

        let registry = orch.execution_controller().trigger_registry();
        registry.register(Arc::new(OneShotReroute {
            source: m_source.id,
            target: m_target.id,
        }));

        let scheduler = MilestoneScheduler::from_orchestrator(&orch);
        let report = scheduler.run_project(project.id).await.unwrap();

        // Walk order: source (rerouted-blocked), then target (Done).
        assert_eq!(report.attempted.len(), 2);
        assert_eq!(report.attempted[0].0, m_source.id);
        assert!(matches!(
            report.attempted[0].1,
            MilestoneOutcome::BlockedBeforeStart { .. }
        ));
        assert_eq!(report.attempted[1].0, m_target.id);
        assert_eq!(report.attempted[1].1, MilestoneOutcome::Done);
        assert_eq!(report.reroutes, vec![(m_source.id, m_target.id)]);

        let source = orch.get_milestone(m_source.id).await.unwrap();
        let target = orch.get_milestone(m_target.id).await.unwrap();
        assert_eq!(source.status, MilestoneStatus::Blocked);
        assert_eq!(target.status, MilestoneStatus::Done);
    }

    /// `AfterMilestone` Reroute jumps the walk to the target *after* the
    /// source milestone has reached `Done`. The source's work is preserved.
    #[tokio::test]
    async fn after_milestone_reroute_redirects_next_walk() {
        let orch = build_orchestrator();
        let project = orch
            .create_project(Goal::new("reroute after", swell_core::TaskId::new()))
            .await;
        let m_source = orch
            .create_milestone(project.id, "source".to_string())
            .await
            .unwrap();
        let m_target = orch
            .create_milestone(project.id, "target".to_string())
            .await
            .unwrap();

        struct AfterReroute {
            source: MilestoneId,
            target: MilestoneId,
        }
        #[async_trait]
        impl Trigger for AfterReroute {
            fn name(&self) -> &'static str {
                "reroute_after"
            }
            fn stages(&self) -> &'static [Stage] {
                &[Stage::AfterMilestone]
            }
            async fn run(&self, ctx: &TriggerContext) -> TriggerOutcome {
                if ctx.milestone == Some(self.source) {
                    TriggerOutcome::Reroute(self.target)
                } else {
                    TriggerOutcome::Continue
                }
            }
        }

        let registry = orch.execution_controller().trigger_registry();
        registry.register(Arc::new(AfterReroute {
            source: m_source.id,
            target: m_target.id,
        }));

        let scheduler = MilestoneScheduler::from_orchestrator(&orch);
        let report = scheduler.run_project(project.id).await.unwrap();

        assert_eq!(report.attempted.len(), 2);
        assert_eq!(report.attempted[0].0, m_source.id);
        assert_eq!(report.attempted[0].1, MilestoneOutcome::Done);
        assert_eq!(report.attempted[1].0, m_target.id);
        assert_eq!(report.attempted[1].1, MilestoneOutcome::Done);
        assert_eq!(report.reroutes, vec![(m_source.id, m_target.id)]);
        let source = orch.get_milestone(m_source.id).await.unwrap();
        assert_eq!(source.status, MilestoneStatus::Done);
    }

    /// When the reroute target is missing (e.g. belongs to a different
    /// project), the scheduler logs and falls back to DAG order rather
    /// than stalling. The source milestone is still recorded as
    /// rerouted, but `forced_next` resolves to `None` and the next ready
    /// milestone is picked normally.
    #[tokio::test]
    async fn reroute_to_unknown_target_falls_back_to_dag_order() {
        let orch = build_orchestrator();
        let project = orch
            .create_project(Goal::new("reroute fallback", swell_core::TaskId::new()))
            .await;
        let m_source = orch
            .create_milestone(project.id, "source".to_string())
            .await
            .unwrap();
        let m_fallback = orch
            .create_milestone(project.id, "fallback".to_string())
            .await
            .unwrap();

        // Reroute to an ID that doesn't exist in this project.
        let bogus = MilestoneId::new();
        struct BogusReroute {
            source: MilestoneId,
            target: MilestoneId,
        }
        #[async_trait]
        impl Trigger for BogusReroute {
            fn name(&self) -> &'static str {
                "reroute_bogus"
            }
            fn stages(&self) -> &'static [Stage] {
                &[Stage::AfterMilestone]
            }
            async fn run(&self, ctx: &TriggerContext) -> TriggerOutcome {
                if ctx.milestone == Some(self.source) {
                    TriggerOutcome::Reroute(self.target)
                } else {
                    TriggerOutcome::Continue
                }
            }
        }

        let registry = orch.execution_controller().trigger_registry();
        registry.register(Arc::new(BogusReroute {
            source: m_source.id,
            target: bogus,
        }));

        let scheduler = MilestoneScheduler::from_orchestrator(&orch);
        let report = scheduler.run_project(project.id).await.unwrap();

        // Both milestones still run; reroute hint is recorded; bogus
        // target is dropped on the floor.
        assert_eq!(report.attempted.len(), 2);
        assert!(report
            .attempted
            .iter()
            .all(|(_, o)| *o == MilestoneOutcome::Done));
        assert_eq!(report.reroutes, vec![(m_source.id, bogus)]);
        // Fallback milestone still completed.
        let fallback = orch.get_milestone(m_fallback.id).await.unwrap();
        assert_eq!(fallback.status, MilestoneStatus::Done);
    }

    /// `OnMilestoneBlocked` Reroute jumps the walk to the recovery target
    /// after the source milestone is `Blocked`. Source stays `Blocked`
    /// (its work was not performed), target runs to `Done`. Setup: a
    /// BeforeMilestone halt blocks the source, then a separate
    /// OnMilestoneBlocked trigger returns Reroute to redirect the walk.
    #[tokio::test]
    async fn on_milestone_blocked_reroute_redirects_scheduler() {
        let orch = build_orchestrator();
        let project = orch
            .create_project(Goal::new("reroute on_blocked", swell_core::TaskId::new()))
            .await;
        let m_source = orch
            .create_milestone(project.id, "source".to_string())
            .await
            .unwrap();
        let m_recovery = orch
            .create_milestone(project.id, "recovery".to_string())
            .await
            .unwrap();

        struct HaltSource {
            source: MilestoneId,
        }
        #[async_trait]
        impl Trigger for HaltSource {
            fn name(&self) -> &'static str {
                "halt_source"
            }
            fn stages(&self) -> &'static [Stage] {
                &[Stage::BeforeMilestone]
            }
            async fn run(&self, ctx: &TriggerContext) -> TriggerOutcome {
                if ctx.milestone == Some(self.source) {
                    TriggerOutcome::Halt("stuck".into())
                } else {
                    TriggerOutcome::Continue
                }
            }
        }

        struct RerouteOnBlocked {
            source: MilestoneId,
            recovery: MilestoneId,
        }
        #[async_trait]
        impl Trigger for RerouteOnBlocked {
            fn name(&self) -> &'static str {
                "reroute_on_blocked"
            }
            fn stages(&self) -> &'static [Stage] {
                &[Stage::OnMilestoneBlocked]
            }
            async fn run(&self, ctx: &TriggerContext) -> TriggerOutcome {
                if ctx.milestone == Some(self.source) {
                    TriggerOutcome::Reroute(self.recovery)
                } else {
                    TriggerOutcome::Continue
                }
            }
        }

        let registry = orch.execution_controller().trigger_registry();
        registry.register(Arc::new(HaltSource {
            source: m_source.id,
        }));
        registry.register(Arc::new(RerouteOnBlocked {
            source: m_source.id,
            recovery: m_recovery.id,
        }));

        let scheduler = MilestoneScheduler::from_orchestrator(&orch);
        let report = scheduler.run_project(project.id).await.unwrap();

        assert_eq!(report.attempted.len(), 2);
        assert_eq!(report.attempted[0].0, m_source.id);
        assert!(matches!(
            report.attempted[0].1,
            MilestoneOutcome::BlockedBeforeStart { .. }
        ));
        assert_eq!(report.attempted[1].0, m_recovery.id);
        assert_eq!(report.attempted[1].1, MilestoneOutcome::Done);
        assert_eq!(report.reroutes, vec![(m_source.id, m_recovery.id)]);

        let source = orch.get_milestone(m_source.id).await.unwrap();
        let recovery = orch.get_milestone(m_recovery.id).await.unwrap();
        assert_eq!(source.status, MilestoneStatus::Blocked);
        assert_eq!(recovery.status, MilestoneStatus::Done);
    }

    /// Counter probe to prove BeforeMilestone fires exactly once per milestone.
    #[tokio::test]
    async fn before_milestone_fires_exactly_once() {
        let orch = build_orchestrator();
        let (project_id, _) = project_with_empty_milestone(&orch).await;

        struct Counter {
            seen: Arc<AtomicUsize>,
        }
        #[async_trait]
        impl Trigger for Counter {
            fn name(&self) -> &'static str {
                "counter"
            }
            fn stages(&self) -> &'static [Stage] {
                &[Stage::BeforeMilestone]
            }
            async fn run(&self, _ctx: &TriggerContext) -> TriggerOutcome {
                self.seen.fetch_add(1, Ordering::SeqCst);
                TriggerOutcome::Continue
            }
        }

        let registry: Arc<TriggerRegistry> = orch.execution_controller().trigger_registry();
        let seen = Arc::new(AtomicUsize::new(0));
        registry.register(Arc::new(Counter {
            seen: Arc::clone(&seen),
        }));

        let scheduler = MilestoneScheduler::from_orchestrator(&orch);
        let _ = scheduler.run_project(project_id).await.unwrap();
        assert_eq!(seen.load(Ordering::SeqCst), 1);
    }
}
