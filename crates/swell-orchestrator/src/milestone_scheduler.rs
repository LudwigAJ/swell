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
//! - Tasks **within a milestone are executed sequentially** today. Parallel
//!   fan-out under `SemaphoreWorkerPool` is the half flagged behind
//!   `milestone.parallel_tasks` in `10_migration_plan.md` and is the natural
//!   follow-up once this sequential walk is proven.
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
//! - `Reroute` outcomes from milestone-stage triggers are logged. The
//!   reroute consumer is a follow-up that needs a stable way to surface the
//!   target milestone from `AfterTask` reports too; both will land together.
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
        };

        // Per-run view of milestone statuses. We refresh from the
        // state-machine each iteration so `set_milestone_status` writes
        // from prior iterations are honored when computing the next ready
        // set.
        let mut handled: HashSet<MilestoneId> = HashSet::new();

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

            let next = milestones.iter().find(|m| {
                !handled.contains(&m.id)
                    && matches!(m.status, MilestoneStatus::Pending | MilestoneStatus::Ready)
                    && m.ready_given(milestones.iter())
            });

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
                self.fire_on_blocked(project_id, milestone_id, reason.clone())
                    .await;
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
                    "BeforeMilestone reroute requested — logged, not yet acted upon"
                );
            }

            orchestrator
                .set_milestone_status(milestone_id, MilestoneStatus::Executing)
                .await?;

            // Run tasks sequentially. Parallel fan-out under
            // SemaphoreWorkerPool is the F-flagged follow-up.
            let mut failure: Option<swell_core::TaskId> = None;
            for task_id in &milestone_clone.tasks {
                match self.controller.execute_task(*task_id).await {
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
                self.fire_on_blocked(project_id, milestone_id, reason).await;
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
                self.fire_on_blocked(project_id, milestone_id, reason).await;
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
                self.fire_on_blocked(project_id, milestone_id, reason.clone())
                    .await;
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
                    "AfterMilestone reroute requested — logged, not yet acted upon"
                );
            }

            orchestrator
                .set_milestone_status(milestone_id, MilestoneStatus::Done)
                .await?;
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
    ) {
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
            }
            Some((name, TriggerOutcome::Reroute(target))) => {
                info!(
                    project_id = %project_id,
                    milestone_id = %milestone_id,
                    trigger = %name,
                    target = %target,
                    reason = %reason,
                    "OnMilestoneBlocked reroute requested (scheduler-handled in follow-up)"
                );
            }
            _ => {
                info!(
                    project_id = %project_id,
                    milestone_id = %milestone_id,
                    reason = %reason,
                    "Milestone blocked"
                );
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
