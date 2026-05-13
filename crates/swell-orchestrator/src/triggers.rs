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
use std::sync::Arc;
use swell_core::{MilestoneId, ProjectId, TaskId};

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
}

impl TriggerContext {
    pub fn for_task(stage: Stage, task: TaskId) -> Self {
        Self {
            stage,
            project: None,
            milestone: None,
            task: Some(task),
        }
    }

    pub fn for_milestone(stage: Stage, project: ProjectId, milestone: MilestoneId) -> Self {
        Self {
            stage,
            project: Some(project),
            milestone: Some(milestone),
            task: None,
        }
    }

    pub fn for_project(stage: Stage, project: ProjectId) -> Self {
        Self {
            stage,
            project: Some(project),
            milestone: None,
            task: None,
        }
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
#[derive(Default, Clone)]
pub struct TriggerRegistry {
    triggers: Vec<Arc<dyn Trigger>>,
}

impl std::fmt::Debug for TriggerRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TriggerRegistry")
            .field("count", &self.triggers.len())
            .field(
                "names",
                &self.triggers.iter().map(|t| t.name()).collect::<Vec<_>>(),
            )
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

    pub fn register(&mut self, trigger: Arc<dyn Trigger>) {
        self.triggers.push(trigger);
    }

    pub fn len(&self) -> usize {
        self.triggers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.triggers.is_empty()
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.triggers.iter().map(|t| t.name()).collect()
    }

    /// Run every trigger registered for `stage` in registration order,
    /// stopping at the first non-`Continue` outcome.
    pub async fn fire(&self, ctx: &TriggerContext) -> FireReport {
        let mut report = FireReport {
            fired: Vec::new(),
            short_circuit: None,
        };
        for trigger in &self.triggers {
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

        let mut registry = TriggerRegistry::new();
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

        let mut registry = TriggerRegistry::new();
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

        let mut registry = TriggerRegistry::new();
        registry.register(t1);

        let ctx = TriggerContext::for_task(Stage::AfterTask, TaskId::new());
        let report = registry.fire(&ctx).await;
        match report.short_circuit {
            Some(("reroute", TriggerOutcome::Reroute(id))) => assert_eq!(id, target),
            other => panic!("expected reroute, got {other:?}"),
        }
    }
}
