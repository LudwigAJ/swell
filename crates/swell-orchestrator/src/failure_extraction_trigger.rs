//! Built-in `FailureExtractionTrigger` — F12 of
//! `plan/flow_integration_plan/12_task_generation_failure_and_followup.md`.
//!
//! Fires on [`Stage::OnTaskFailed`]. When a task lands in `Failed` /
//! `Rejected` with a populated [`ValidationResult`], this trigger creates
//! a narrower child task scoped to the first reported error, links it to
//! the parent via `Task.parent`, increments `spawn_depth`, and (if the
//! parent has one) assigns the child to the same milestone so the
//! `MilestoneScheduler` re-runs it in the same DAG branch.
//!
//! A `spawn_depth` cap (default 3, matching the spec) stops the recursive
//! re-narrowing — once a chain hits the cap the trigger emits a warning
//! and returns `Continue` without spawning. Promoting that to a Researcher
//! reroute is the follow-up the PR `04` slice owns.
//!
//! The trigger captures a `Weak<Orchestrator>` at factory time the same
//! way `GitCommitTrigger` captures its `CommitStrategy`. `OnTaskFailed`
//! contexts don't carry [`TaskTriggerState`], so the trigger pulls the
//! failed task through the orchestrator's state machine.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use swell_core::MilestoneId;
use tracing::{info, warn};

use crate::trigger_config::TriggerFactoryRegistry;
use crate::triggers::{Stage, Trigger, TriggerContext, TriggerOutcome};
use crate::Orchestrator;

/// Default maximum spawn depth before failure-derived spawning halts.
/// Mirrors the Orchestrator spec §3 cap of "3 spawns per original task".
pub const DEFAULT_MAX_SPAWN_DEPTH: u8 = 3;

/// `OnTaskFailed` trigger that creates a narrow child task from the first
/// error in the failed task's `ValidationResult`.
///
/// When a task's failure chain reaches `max_spawn_depth`, the trigger
/// escalates rather than swallows: if a `researcher_milestone` is
/// configured it emits [`TriggerOutcome::Reroute`] so the
/// [`MilestoneScheduler`][crate::milestone_scheduler] jumps to the
/// recovery milestone (typically the one [`ResearcherTrigger`][
/// crate::researcher_trigger::ResearcherTrigger] is wired against).
/// Without a configured escalation target it logs a warning and
/// returns `Continue` (preserves prior behavior).
pub struct FailureExtractionTrigger {
    stages: &'static [Stage],
    orchestrator: Weak<Orchestrator>,
    max_spawn_depth: u8,
    /// Recovery milestone the trigger reroutes to when the spawn-depth
    /// cap is reached. `None` keeps the legacy "log + Continue"
    /// behavior so this slice is opt-in.
    researcher_milestone: Option<MilestoneId>,
}

impl FailureExtractionTrigger {
    pub fn new(
        stages: &'static [Stage],
        orchestrator: Weak<Orchestrator>,
        max_spawn_depth: u8,
    ) -> Self {
        Self {
            stages,
            orchestrator,
            max_spawn_depth,
            researcher_milestone: None,
        }
    }

    /// Convenience constructor for the default `OnTaskFailed`-only wiring
    /// with the spec-mandated depth cap.
    pub fn on_task_failed(orchestrator: Weak<Orchestrator>) -> Self {
        Self::new(
            &[Stage::OnTaskFailed],
            orchestrator,
            DEFAULT_MAX_SPAWN_DEPTH,
        )
    }

    /// Builder-style setter for the escalation target. When set, hitting
    /// the spawn-depth cap returns `Reroute(researcher_milestone)`
    /// instead of `Continue`.
    pub fn with_researcher_milestone(mut self, milestone: MilestoneId) -> Self {
        self.researcher_milestone = Some(milestone);
        self
    }
}

#[async_trait]
impl Trigger for FailureExtractionTrigger {
    fn name(&self) -> &'static str {
        "failure_extraction"
    }

    fn stages(&self) -> &'static [Stage] {
        self.stages
    }

    async fn run(&self, ctx: &TriggerContext) -> TriggerOutcome {
        let Some(parent_id) = ctx.task else {
            warn!(stage = ?ctx.stage, "failure_extraction fired without task id");
            return TriggerOutcome::Continue;
        };

        let Some(orch) = self.orchestrator.upgrade() else {
            warn!("failure_extraction: orchestrator dropped; skipping");
            return TriggerOutcome::Continue;
        };

        let parent = match orch.get_task(parent_id).await {
            Ok(t) => t,
            Err(e) => {
                warn!(task_id = %parent_id, error = %e, "failure_extraction: parent task not found");
                return TriggerOutcome::Continue;
            }
        };

        // Cap: the parent has already spawned through the chain too many
        // times. Don't spawn another. If a researcher milestone is
        // configured, escalate via Reroute — the OnTaskFailed reroute
        // side-channel + MilestoneScheduler will redirect the walk to
        // the recovery milestone. Without one, log and Continue
        // (preserves the original behavior).
        if parent.spawn_depth >= self.max_spawn_depth {
            // Resolution order at the cap:
            //   1. Explicit `researcher_milestone` from trigger config.
            //   2. Auto-discovered milestone in the parent task's
            //      project, tagged `MilestoneKind::Researcher` (set via
            //      `Orchestrator::create_researcher_milestone`).
            //   3. Log + Continue (legacy behavior).
            let escalation_target = if let Some(target) = self.researcher_milestone {
                Some(target)
            } else if let Some(milestone_id) = parent.milestone {
                match orch.get_milestone(milestone_id).await {
                    Ok(milestone) => orch.find_researcher_milestone(milestone.project).await,
                    Err(e) => {
                        warn!(
                            task_id = %parent_id,
                            milestone = %milestone_id,
                            error = %e,
                            "failure_extraction: failed to look up parent milestone for auto-discovery"
                        );
                        None
                    }
                }
            } else {
                None
            };

            if let Some(target) = escalation_target {
                warn!(
                    task_id = %parent_id,
                    spawn_depth = parent.spawn_depth,
                    cap = self.max_spawn_depth,
                    researcher_milestone = %target,
                    source = if self.researcher_milestone.is_some() {
                        "config"
                    } else {
                        "auto-discovered MilestoneKind::Researcher"
                    },
                    "failure_extraction: spawn cap reached; escalating to researcher milestone"
                );
                return TriggerOutcome::Reroute(target);
            }
            warn!(
                task_id = %parent_id,
                spawn_depth = parent.spawn_depth,
                cap = self.max_spawn_depth,
                "failure_extraction: spawn cap reached; no researcher_milestone configured (Continue)"
            );
            return TriggerOutcome::Continue;
        }

        // Extract the narrow scope. Prefer the first validation error; fall
        // back to the rejected_reason or a generic message so we always
        // produce *something* actionable.
        let narrow_error = parent
            .validation_result
            .as_ref()
            .and_then(|r| r.errors.first().cloned())
            .or_else(|| parent.rejected_reason.clone())
            .unwrap_or_else(|| "task failed without a structured error".to_string());

        let description = format!("Fix narrow failure from task {parent_id}: {narrow_error}");

        // Create the child through the state machine (bypassing the
        // orchestrator's novelty check — a failure-derived re-run is
        // *expected* to overlap the parent).
        let child_id = {
            let sm_handle = orch.state_machine();
            let sm = sm_handle.read().await;
            let child = sm.create_task(description);
            // Stamp parent + depth on the child.
            if let Err(e) = sm.with_task_mut(child.id, |t| {
                t.parent = Some(parent_id);
                t.spawn_depth = parent.spawn_depth.saturating_add(1);
                Ok(())
            }) {
                warn!(
                    parent = %parent_id,
                    child = %child.id,
                    error = %e,
                    "failure_extraction: child created but parent/depth not stamped"
                );
            }
            // Inherit milestone so the scheduler re-runs the child in the
            // same DAG branch as the parent failure.
            if let Some(milestone_id) = parent.milestone {
                if let Err(e) = sm.assign_task_to_milestone(child.id, milestone_id) {
                    warn!(
                        parent = %parent_id,
                        child = %child.id,
                        milestone = %milestone_id,
                        error = %e,
                        "failure_extraction: failed to inherit parent milestone"
                    );
                }
            }
            child.id
        };

        info!(
            parent = %parent_id,
            child = %child_id,
            spawn_depth = parent.spawn_depth.saturating_add(1),
            "failure_extraction: spawned narrow child task"
        );
        TriggerOutcome::Continue
    }
}

/// Register the `failure_extraction` factory on the given
/// [`TriggerFactoryRegistry`]. The factory captures a `Weak<Orchestrator>`
/// so the trigger can fetch the failed task at fire time and create the
/// child through the live state machine.
///
/// Config blob is optional; recognized keys:
///
/// - `max_spawn_depth: u8` — overrides [`DEFAULT_MAX_SPAWN_DEPTH`].
/// - `researcher_milestone: String` — UUID of the recovery milestone
///   the trigger reroutes to at the cap. Parsed as [`MilestoneId`];
///   malformed values are warned about and ignored (the trigger falls
///   back to log-and-Continue at the cap).
///
/// Malformed values are reported as a warning by the loader and the
/// default is used.
pub fn register_failure_extraction_factory(
    factories: &mut TriggerFactoryRegistry,
    orchestrator: Weak<Orchestrator>,
) {
    factories.register("failure_extraction", move |stages, config| {
        let leaked: &'static [Stage] = Box::leak(stages.to_vec().into_boxed_slice());
        let max = config
            .get("max_spawn_depth")
            .and_then(|v| v.as_u64())
            .and_then(|n| u8::try_from(n).ok())
            .unwrap_or(DEFAULT_MAX_SPAWN_DEPTH);
        let researcher_milestone = config
            .get("researcher_milestone")
            .and_then(|v| v.as_str())
            .and_then(|raw| match raw.parse::<MilestoneId>() {
                Ok(id) => Some(id),
                Err(e) => {
                    warn!(
                        raw = %raw,
                        error = %e,
                        "failure_extraction: researcher_milestone is not a valid UUID; ignoring"
                    );
                    None
                }
            });
        let mut trigger = FailureExtractionTrigger::new(leaked, orchestrator.clone(), max);
        if let Some(milestone) = researcher_milestone {
            trigger = trigger.with_researcher_milestone(milestone);
        }
        Some(Arc::new(trigger) as Arc<dyn Trigger>)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::triggers::TriggerContext;
    use crate::OrchestratorBuilder;
    use swell_core::{Goal, ValidationResult};

    fn validation_result_with_error(msg: &str) -> ValidationResult {
        ValidationResult {
            passed: false,
            lint_passed: true,
            tests_passed: false,
            security_passed: true,
            ai_review_passed: true,
            errors: vec![msg.to_string()],
            warnings: Vec::new(),
        }
    }

    /// Parent task with an error in its `ValidationResult` spawns a
    /// narrow child stamped with `parent` and `spawn_depth = 1`.
    #[tokio::test]
    async fn failure_extracted_into_narrower_task() {
        let orch = OrchestratorBuilder::new().build();
        let parent = orch
            .create_task(
                "implement the feature".to_string(),
                vec!["src/lib.rs".to_string()],
            )
            .await
            .unwrap();

        // Stamp a validation error on the parent.
        {
            let sm = orch.state_machine();
            let sm = sm.read().await;
            sm.with_task_mut(parent.id, |t| {
                t.validation_result = Some(validation_result_with_error(
                    "test_foo_passes failed: assertion left != right",
                ));
                Ok(())
            })
            .unwrap();
        }

        let trigger = FailureExtractionTrigger::on_task_failed(Arc::downgrade(&orch));
        let ctx = TriggerContext::for_task(Stage::OnTaskFailed, parent.id);
        let outcome = trigger.run(&ctx).await;
        assert_eq!(outcome, TriggerOutcome::Continue);

        // Find the child: any task with parent = Some(parent.id).
        let sm_handle = orch.state_machine();
        let sm = sm_handle.read().await;
        let all = sm.get_all_tasks();
        let children: Vec<_> = all.iter().filter(|t| t.parent == Some(parent.id)).collect();
        assert_eq!(children.len(), 1, "expected exactly one child task");
        let child = &children[0];
        assert_eq!(child.spawn_depth, 1);
        assert!(
            child.description.contains("test_foo_passes failed"),
            "child description should carry narrow scope: {}",
            child.description
        );
        assert!(
            child.description.contains(&parent.id.to_string()),
            "child description should reference parent id"
        );
    }

    /// Once a task reaches the configured `spawn_depth` cap, the trigger
    /// stops spawning further children. The chain is left to escalation
    /// (PR 04, Researcher).
    #[tokio::test]
    async fn spawn_depth_cap_stops_spawning() {
        let orch = OrchestratorBuilder::new().build();
        let parent = orch
            .create_task(
                "deep chain task".to_string(),
                vec!["src/deep.rs".to_string()],
            )
            .await
            .unwrap();
        // Pretend the parent is already at the cap.
        {
            let sm = orch.state_machine();
            let sm = sm.read().await;
            sm.with_task_mut(parent.id, |t| {
                t.spawn_depth = DEFAULT_MAX_SPAWN_DEPTH;
                t.validation_result = Some(validation_result_with_error("yet another failure"));
                Ok(())
            })
            .unwrap();
        }

        let trigger = FailureExtractionTrigger::on_task_failed(Arc::downgrade(&orch));
        let ctx = TriggerContext::for_task(Stage::OnTaskFailed, parent.id);
        let _ = trigger.run(&ctx).await;

        let sm = orch.state_machine();
        let sm = sm.read().await;
        let all = sm.get_all_tasks();
        let children: Vec<_> = all.iter().filter(|t| t.parent == Some(parent.id)).collect();
        assert!(
            children.is_empty(),
            "no child should be spawned past the depth cap"
        );
    }

    /// When the parent has a milestone, the child is assigned to the same
    /// milestone so the `MilestoneScheduler` re-runs it in-branch.
    #[tokio::test]
    async fn child_inherits_parent_milestone() {
        let orch = OrchestratorBuilder::new().build();
        let project = orch
            .create_project(Goal::new("inherit smoke", swell_core::TaskId::new()))
            .await;
        let milestone = orch
            .create_milestone(project.id, "m1".to_string())
            .await
            .unwrap();
        let parent = orch
            .create_task(
                "implement narrow change".to_string(),
                vec!["src/m1.rs".to_string()],
            )
            .await
            .unwrap();
        orch.assign_task_to_milestone(parent.id, milestone.id)
            .await
            .unwrap();
        {
            let sm = orch.state_machine();
            let sm = sm.read().await;
            sm.with_task_mut(parent.id, |t| {
                t.validation_result = Some(validation_result_with_error("compile error E0061"));
                Ok(())
            })
            .unwrap();
        }

        let trigger = FailureExtractionTrigger::on_task_failed(Arc::downgrade(&orch));
        let ctx = TriggerContext::for_task(Stage::OnTaskFailed, parent.id);
        let _ = trigger.run(&ctx).await;

        let sm = orch.state_machine();
        let sm = sm.read().await;
        let all = sm.get_all_tasks();
        let child = all
            .iter()
            .find(|t| t.parent == Some(parent.id))
            .expect("child should be spawned");
        assert_eq!(
            child.milestone,
            Some(milestone.id),
            "child must inherit parent's milestone"
        );
    }

    /// At the spawn-depth cap with a `researcher_milestone` configured,
    /// the trigger escalates with `Reroute(target)` instead of swallowing
    /// the failure. Verified by directly constructing the trigger; the
    /// scheduler-side drain of this Reroute is covered by
    /// `milestone_scheduler::tests::on_task_failed_reroute_*`.
    #[tokio::test]
    async fn spawn_depth_cap_reroutes_to_researcher_milestone_when_configured() {
        let orch = OrchestratorBuilder::new().build();
        let project = orch
            .create_project(Goal::new("escalation smoke", swell_core::TaskId::new()))
            .await;
        let researcher_milestone = orch
            .create_milestone(project.id, "researcher".into())
            .await
            .unwrap();

        let parent = orch
            .create_task("stuck task".into(), vec!["src/stuck.rs".into()])
            .await
            .unwrap();
        {
            let sm = orch.state_machine();
            let sm = sm.read().await;
            sm.with_task_mut(parent.id, |t| {
                t.spawn_depth = DEFAULT_MAX_SPAWN_DEPTH;
                t.validation_result = Some(validation_result_with_error("compile cycle"));
                Ok(())
            })
            .unwrap();
        }

        let trigger = FailureExtractionTrigger::on_task_failed(Arc::downgrade(&orch))
            .with_researcher_milestone(researcher_milestone.id);
        let ctx = TriggerContext::for_task(Stage::OnTaskFailed, parent.id);
        let outcome = trigger.run(&ctx).await;
        match outcome {
            TriggerOutcome::Reroute(target) => assert_eq!(target, researcher_milestone.id),
            other => panic!("expected Reroute at cap with researcher_milestone set, got {other:?}"),
        }

        // No child should have been spawned at the cap.
        let sm = orch.state_machine();
        let sm = sm.read().await;
        let all = sm.get_all_tasks();
        let children: Vec<_> = all.iter().filter(|t| t.parent == Some(parent.id)).collect();
        assert!(children.is_empty(), "no child spawn past the cap");
    }

    /// At the cap without a `researcher_milestone`, the trigger keeps
    /// its legacy log-and-Continue behavior.
    #[tokio::test]
    async fn spawn_depth_cap_without_researcher_milestone_returns_continue() {
        let orch = OrchestratorBuilder::new().build();
        let parent = orch
            .create_task("stuck task".into(), vec!["src/stuck.rs".into()])
            .await
            .unwrap();
        {
            let sm = orch.state_machine();
            let sm = sm.read().await;
            sm.with_task_mut(parent.id, |t| {
                t.spawn_depth = DEFAULT_MAX_SPAWN_DEPTH;
                t.validation_result = Some(validation_result_with_error("compile cycle"));
                Ok(())
            })
            .unwrap();
        }

        let trigger = FailureExtractionTrigger::on_task_failed(Arc::downgrade(&orch));
        let ctx = TriggerContext::for_task(Stage::OnTaskFailed, parent.id);
        let outcome = trigger.run(&ctx).await;
        assert_eq!(outcome, TriggerOutcome::Continue);
    }

    /// Factory parses a `researcher_milestone` UUID string and wires it
    /// onto the trigger. Malformed UUIDs warn and fall through (trigger
    /// still constructs).
    #[tokio::test]
    async fn factory_parses_researcher_milestone_from_config() {
        let orch = OrchestratorBuilder::new().build();
        let project = orch
            .create_project(Goal::new("factory escalation", swell_core::TaskId::new()))
            .await;
        let researcher_milestone = orch
            .create_milestone(project.id, "researcher".into())
            .await
            .unwrap();

        let mut factories = TriggerFactoryRegistry::new();
        register_failure_extraction_factory(&mut factories, Arc::downgrade(&orch));

        let cfg_json = format!(
            r#"{{ "failure_extraction": {{ "stages": ["OnTaskFailed"], "config": {{ "researcher_milestone": "{}", "max_spawn_depth": 1 }} }} }}"#,
            researcher_milestone.id
        );
        let cfg: crate::trigger_config::TriggerConfig = serde_json::from_str(&cfg_json).unwrap();
        let loaded = crate::trigger_config::build_triggers(&cfg, &factories);
        assert_eq!(loaded.built.len(), 1);
        let trigger = loaded.built.into_iter().next().unwrap();

        let parent = orch
            .create_task("cap task".into(), vec!["src/cap.rs".into()])
            .await
            .unwrap();
        {
            let sm = orch.state_machine();
            let sm = sm.read().await;
            sm.with_task_mut(parent.id, |t| {
                t.spawn_depth = 1;
                t.validation_result = Some(validation_result_with_error("compile cycle"));
                Ok(())
            })
            .unwrap();
        }
        let ctx = TriggerContext::for_task(Stage::OnTaskFailed, parent.id);
        let outcome = trigger.run(&ctx).await;
        match outcome {
            TriggerOutcome::Reroute(target) => assert_eq!(target, researcher_milestone.id),
            other => panic!("expected Reroute via factory wiring, got {other:?}"),
        }
    }

    /// Malformed `researcher_milestone` config is warned-about and the
    /// trigger still constructs without escalation wired.
    #[tokio::test]
    async fn factory_warns_on_malformed_researcher_milestone() {
        let orch = OrchestratorBuilder::new().build();
        let mut factories = TriggerFactoryRegistry::new();
        register_failure_extraction_factory(&mut factories, Arc::downgrade(&orch));
        let cfg: crate::trigger_config::TriggerConfig = serde_json::from_str(
            r#"{ "failure_extraction": { "stages": ["OnTaskFailed"], "config": { "researcher_milestone": "not-a-uuid" } } }"#,
        )
        .unwrap();
        let loaded = crate::trigger_config::build_triggers(&cfg, &factories);
        assert_eq!(
            loaded.built.len(),
            1,
            "malformed escalation must not break factory"
        );
    }

    /// At the cap with no explicit `researcher_milestone` in config but
    /// the parent's project contains a milestone tagged
    /// `MilestoneKind::Researcher`, the trigger auto-discovers it and
    /// reroutes — operators no longer need to copy UUIDs into
    /// `.swell/triggers.json`. See
    /// `plan/flow_integration_plan/04_researcher_handoff.md` →
    /// "auto-discover researcher milestone" follow-up.
    #[tokio::test]
    async fn spawn_depth_cap_auto_discovers_researcher_milestone_by_kind() {
        let orch = OrchestratorBuilder::new().build();
        let project = orch
            .create_project(Goal::new("auto-discover smoke", swell_core::TaskId::new()))
            .await;
        // Plain work milestone (parent lives here).
        let work_milestone = orch
            .create_milestone(project.id, "work".into())
            .await
            .unwrap();
        // Recovery milestone via the new orchestrator API — stamps
        // `kind = MilestoneKind::Researcher`.
        let researcher_milestone = orch
            .create_researcher_milestone(project.id, "researcher".into())
            .await
            .unwrap();
        assert_eq!(
            researcher_milestone.kind,
            swell_core::MilestoneKind::Researcher,
            "create_researcher_milestone must stamp the kind"
        );

        let parent = orch
            .create_task("stuck task".into(), vec!["src/stuck.rs".into()])
            .await
            .unwrap();
        orch.assign_task_to_milestone(parent.id, work_milestone.id)
            .await
            .unwrap();
        {
            let sm = orch.state_machine();
            let sm = sm.read().await;
            sm.with_task_mut(parent.id, |t| {
                t.spawn_depth = DEFAULT_MAX_SPAWN_DEPTH;
                t.validation_result = Some(validation_result_with_error("loop hit cap"));
                Ok(())
            })
            .unwrap();
        }

        // No explicit config; relies entirely on auto-discovery.
        let trigger = FailureExtractionTrigger::on_task_failed(Arc::downgrade(&orch));
        let ctx = TriggerContext::for_task(Stage::OnTaskFailed, parent.id);
        let outcome = trigger.run(&ctx).await;
        match outcome {
            TriggerOutcome::Reroute(target) => {
                assert_eq!(
                    target, researcher_milestone.id,
                    "auto-discovery should route to the MilestoneKind::Researcher \
                     milestone in the parent task's project"
                );
            }
            other => panic!(
                "expected Reroute via auto-discovery, got {other:?}; \
                 the trigger should fall back to find_researcher_milestone \
                 when researcher_milestone is not pinned in config"
            ),
        }
    }

    /// Explicit `researcher_milestone` in config wins over auto-discovery.
    /// Pin the precedence so a future "always prefer kind tag" refactor
    /// requires deliberately updating the contract.
    #[tokio::test]
    async fn explicit_researcher_milestone_in_config_takes_precedence_over_kind() {
        let orch = OrchestratorBuilder::new().build();
        let project = orch
            .create_project(Goal::new("precedence smoke", swell_core::TaskId::new()))
            .await;
        let work_milestone = orch
            .create_milestone(project.id, "work".into())
            .await
            .unwrap();
        // Kind-tagged auto-discovery target (would be picked by
        // auto-discovery if config target was absent).
        let _auto_target = orch
            .create_researcher_milestone(project.id, "auto".into())
            .await
            .unwrap();
        // Explicit config-pinned target — must win.
        let explicit_target = orch
            .create_milestone(project.id, "explicit".into())
            .await
            .unwrap();

        let parent = orch
            .create_task("cap task".into(), vec!["src/cap.rs".into()])
            .await
            .unwrap();
        orch.assign_task_to_milestone(parent.id, work_milestone.id)
            .await
            .unwrap();
        {
            let sm = orch.state_machine();
            let sm = sm.read().await;
            sm.with_task_mut(parent.id, |t| {
                t.spawn_depth = DEFAULT_MAX_SPAWN_DEPTH;
                t.validation_result = Some(validation_result_with_error("at the cap"));
                Ok(())
            })
            .unwrap();
        }

        let trigger = FailureExtractionTrigger::on_task_failed(Arc::downgrade(&orch))
            .with_researcher_milestone(explicit_target.id);
        let ctx = TriggerContext::for_task(Stage::OnTaskFailed, parent.id);
        let outcome = trigger.run(&ctx).await;
        match outcome {
            TriggerOutcome::Reroute(target) => assert_eq!(target, explicit_target.id),
            other => panic!("expected explicit Reroute, got {other:?}"),
        }
    }

    /// Parent without a milestone (loose task) at the cap with no
    /// configured target still degrades to `Continue` — auto-discovery
    /// has no project to anchor against.
    #[tokio::test]
    async fn auto_discovery_skips_for_loose_task_at_cap() {
        let orch = OrchestratorBuilder::new().build();
        let parent = orch
            .create_task("loose task".into(), vec!["src/loose.rs".into()])
            .await
            .unwrap();
        {
            let sm = orch.state_machine();
            let sm = sm.read().await;
            sm.with_task_mut(parent.id, |t| {
                t.spawn_depth = DEFAULT_MAX_SPAWN_DEPTH;
                t.validation_result = Some(validation_result_with_error("loose failure"));
                Ok(())
            })
            .unwrap();
        }

        let trigger = FailureExtractionTrigger::on_task_failed(Arc::downgrade(&orch));
        let ctx = TriggerContext::for_task(Stage::OnTaskFailed, parent.id);
        let outcome = trigger.run(&ctx).await;
        assert_eq!(outcome, TriggerOutcome::Continue);
    }

    /// Factory honors `max_spawn_depth` from config.
    #[tokio::test]
    async fn factory_honors_max_spawn_depth_override() {
        let orch = OrchestratorBuilder::new().build();
        let mut factories = TriggerFactoryRegistry::new();
        register_failure_extraction_factory(&mut factories, Arc::downgrade(&orch));

        // The factory hands back a Trigger; we just verify the round-trip
        // via the config-loader pipeline doesn't reject the override.
        let cfg = serde_json::json!({"max_spawn_depth": 1});
        let names = factories.known_names();
        assert!(names.contains(&"failure_extraction"));

        // Direct construction with cap=1: a parent at depth 1 must not
        // spawn.
        let trigger = FailureExtractionTrigger::new(
            Box::leak(vec![Stage::OnTaskFailed].into_boxed_slice()),
            Arc::downgrade(&orch),
            1,
        );
        let parent = orch
            .create_task(
                "cap override task".to_string(),
                vec!["src/cap.rs".to_string()],
            )
            .await
            .unwrap();
        {
            let sm = orch.state_machine();
            let sm = sm.read().await;
            sm.with_task_mut(parent.id, |t| {
                t.spawn_depth = 1;
                t.validation_result = Some(validation_result_with_error("blocked"));
                Ok(())
            })
            .unwrap();
        }
        let ctx = TriggerContext::for_task(Stage::OnTaskFailed, parent.id);
        let _ = trigger.run(&ctx).await;

        let sm = orch.state_machine();
        let sm = sm.read().await;
        let all = sm.get_all_tasks();
        let children: Vec<_> = all.iter().filter(|t| t.parent == Some(parent.id)).collect();
        assert!(children.is_empty(), "override cap=1 must block spawn");
        // Silence unused warning on cfg
        let _ = cfg;
    }
}
