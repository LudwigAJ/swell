//! Built-in `ResearcherTrigger` — PR 04 of
//! `plan/flow_integration_plan/04_researcher_handoff.md`.
//!
//! When a task or milestone is stuck (failed validation past retry,
//! milestone blocked by an upstream halt), the Researcher trigger
//! invokes a diagnostic `DiagnosticResearcher` to decide what to do
//! and translates its [`Handoff`] verdict into a [`TriggerOutcome`].
//!
//! ## What ships
//!
//! - The trigger itself: lifecycle, budget guardrail, milestone
//!   resolution, [`Handoff`] → [`TriggerOutcome`] translation.
//! - [`StubDiagnosticResearcher`] for tests / for the default daemon
//!   wiring (returns `Continue` so installing the trigger is a no-op
//!   behavior change).
//! - [`LlmDiagnosticResearcher`] — the real backend. Builds a system +
//!   user prompt from [`DiagnosticContext`], calls a configured
//!   `LlmBackend`, parses a single-JSON verdict, and falls back to
//!   `Handoff::Continue` on any LLM/parse failure (a broken diagnostic
//!   must never make recovery worse than not running it).
//! - Factory registration through [`TriggerFactoryRegistry`] for both
//!   the stub ([`register_default_researcher_factory`]) and the live
//!   LLM ([`register_llm_researcher_factory`]) variants, so
//!   `.swell/triggers.json` can opt in.
//!
//! ## Deferred (separate slices)
//!
//! - Read-only tool access for the diagnostic (`read_file`, `search`,
//!   `web_fetch`, memory query). Today the prompt summarizes the
//!   failure inline; tool-loop diagnosis is the natural follow-up.
//! - `LoopIntervention::Escalation` → `OnTaskFailed` plumbing.
//! - `Handoff::SplitMilestone` actually creating new milestones (needs
//!   a milestone factory the orchestrator doesn't expose yet).
//!
//! ## Translator semantics
//!
//! | [`Handoff`]                    | [`TriggerOutcome`] |
//! |--------------------------------|--------------------|
//! | `Replan { milestone, .. }`     | `Reroute(milestone)` |
//! | `Abandon { reason, .. }`       | `Halt(reason)` |
//! | `SplitMilestone { .. }`        | `Continue` (logged; impl deferred) |
//! | `Continue`                     | `Continue` |
//!
//! ## Budget
//!
//! Each milestone has an invocation counter. Once it reaches
//! [`ResearcherBudget::cap`], further fires for that milestone return
//! `Halt("researcher budget exceeded")` *without* invoking the
//! diagnostic. The counter is incremented even when the diagnostic
//! returns `Continue` — the cap is about how many *researcher calls*
//! we make on one milestone, not how many useful recoveries we get.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use async_trait::async_trait;
use serde::Deserialize;
use swell_core::{
    LlmBackend, LlmConfig, LlmMessage, LlmRole, LlmToolCall, LlmToolDefinition, MilestoneId,
    ProjectId, Task, TaskId, ToolOutput, ToolResultContent,
};
use swell_tools::ToolRegistry;
use tracing::{info, warn};

use crate::trigger_config::TriggerFactoryRegistry;
use crate::triggers::{Stage, Trigger, TriggerContext, TriggerOutcome};
use crate::Orchestrator;

/// Default per-milestone researcher invocation cap, per
/// `04_researcher_handoff.md`. The second invocation auto-Halts the
/// milestone to prevent infinite re-plan loops.
pub const DEFAULT_RESEARCHER_INVOCATION_CAP: usize = 2;

/// Verdict the diagnostic returns. The [`ResearcherTrigger`] translates
/// this into a [`TriggerOutcome`] the milestone scheduler can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handoff {
    /// Re-plan the *existing* `milestone` (typically the source itself
    /// after the diagnostic re-prompted the planner). Translates to
    /// `Reroute(milestone)` — the scheduler will pick up the milestone
    /// on its next walk iteration.
    Replan {
        milestone: MilestoneId,
        reason: String,
    },
    /// Split the source milestone into the provided sub-plans.
    /// Translates to `Continue` for now — actually creating the new
    /// milestones is deferred until the orchestrator exposes a
    /// milestone factory beyond `create_milestone(name)`.
    SplitMilestone {
        milestone: MilestoneId,
        into: Vec<MilestonePlan>,
    },
    /// Give up on this milestone. Translates to `Halt(reason)`.
    Abandon {
        milestone: MilestoneId,
        reason: String,
    },
    /// Researcher had no actionable recommendation. Translates to
    /// `Continue`; the scheduler proceeds with whatever the failing
    /// path was already going to do.
    Continue,
}

/// Spec for one child milestone the `ResearcherTrigger` will create
/// when applying `Handoff::SplitMilestone`. Re-exported from
/// [`swell_core::MilestonePlan`] so the LLM wire format and
/// [`Orchestrator::split_milestone`] share one type.
pub use swell_core::MilestonePlan;

/// Context handed to the diagnostic at fire time. Built by
/// [`ResearcherTrigger::build_context`] from the trigger payload + the
/// live orchestrator.
#[derive(Debug, Clone)]
pub struct DiagnosticContext {
    /// Lifecycle stage that fired the trigger.
    pub stage: Stage,
    /// Project the affected milestone belongs to, if known. May be
    /// `None` when fired from `OnTaskFailed` and the failing task has
    /// no milestone (loose task).
    pub project: Option<ProjectId>,
    /// Milestone the diagnostic should reason about. Always present —
    /// the trigger refuses to invoke the diagnostic without a
    /// milestone target.
    pub milestone: MilestoneId,
    /// Failing task, if the fire was task-scoped. `None` for
    /// milestone-scoped fires (`OnMilestoneBlocked`).
    pub failing_task: Option<Task>,
    /// Free-form reason the orchestrator surfaced when the milestone
    /// went blocked / the task failed. Best-effort — may be empty.
    pub reason: String,
    /// Current invocation count *after* the budget pre-check passes
    /// but *before* this fire is counted. Lets diagnostics know how
    /// many times they have been tried already on this milestone.
    pub prior_invocations: usize,
    /// True when this fire was driven by a
    /// [`crate::loop_detection::LoopIntervention::Escalation`] from
    /// the generator's tool loop, rather than by a validation failure
    /// or BeforeTask halt. Propagated from `TriggerContext.escalation`
    /// so the LLM diagnostic can prompt differently on doom loops.
    /// Default `false` for milestone-scoped fires.
    pub escalation: bool,
}

/// Plug-in point for the actual diagnostic. Real impls call an LLM with
/// read-only tools; the test impl just returns a queued [`Handoff`].
#[async_trait]
pub trait DiagnosticResearcher: Send + Sync {
    async fn diagnose(&self, ctx: &DiagnosticContext) -> Handoff;
}

/// No-op diagnostic — always returns `Handoff::Continue`. This is the
/// default the daemon installs so `.swell/triggers.json` can enable
/// the trigger registration without changing observable behavior
/// (matches the F3/F4/F9 default-on-without-behavior-change pattern).
#[derive(Debug, Default)]
pub struct StubDiagnosticResearcher {
    queue: Mutex<Vec<Handoff>>,
}

impl StubDiagnosticResearcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a sequence of handoff verdicts to return on successive
    /// `diagnose` calls. Once exhausted, falls back to
    /// `Handoff::Continue`.
    pub fn with_queue(handoffs: Vec<Handoff>) -> Self {
        Self {
            queue: Mutex::new(handoffs),
        }
    }

    pub fn push(&self, handoff: Handoff) {
        self.queue
            .lock()
            .expect("stub queue poisoned")
            .push(handoff);
    }
}

#[async_trait]
impl DiagnosticResearcher for StubDiagnosticResearcher {
    async fn diagnose(&self, _ctx: &DiagnosticContext) -> Handoff {
        let mut q = self.queue.lock().expect("stub queue poisoned");
        if q.is_empty() {
            Handoff::Continue
        } else {
            q.remove(0)
        }
    }
}

/// Default `max_tokens` budget for the diagnostic LLM call. The response
/// is a short JSON verdict, not free-form prose; 1024 leaves comfortable
/// headroom for the reason field.
pub const DEFAULT_LLM_RESEARCHER_MAX_TOKENS: u64 = 1024;

/// Default temperature for the diagnostic. We want deterministic
/// verdicts on identical inputs, not creative re-plans, so we run cold.
pub const DEFAULT_LLM_RESEARCHER_TEMPERATURE: f32 = 0.0;

/// Default cap on tool-call iterations the diagnostic is allowed to run
/// before being forced to emit a verdict. The Researcher budget caps
/// *invocations* of the trigger; this caps tool calls *within* a single
/// invocation, so a chatty model can't burn an unbounded number of
/// `read_file` / `search` calls on one milestone.
pub const DEFAULT_LLM_RESEARCHER_MAX_TOOL_ITERATIONS: usize = 5;

/// System prompt — sets the diagnostic's role and pins the response
/// schema. Used when the diagnostic is configured *without* a tool
/// registry: the model gets *one* instruction and must reply with one
/// JSON object. No prose, no markdown fences.
const DIAGNOSTIC_SYSTEM_PROMPT: &str = r#"You are a senior software engineer diagnosing why a coding task or milestone is stuck.

Your job is to pick exactly one recovery action for the orchestrator:

- "replan"          → the milestone should be re-attempted, optionally pointing the scheduler at a *different* milestone id known to the orchestrator. Use when the failure looks recoverable with a fresh plan.
- "abandon"         → the milestone is unsalvageable in this run; the scheduler should halt this branch.
- "split_milestone" → the milestone is too large; break it into a sequential chain of smaller sub-milestones. The orchestrator will create each entry in `sub_plans` as a new milestone, rewire the DAG, and walk into the first child. Use when one focused milestone became too broad.
- "continue"        → no useful action; let the existing failure path proceed.

Respond with a SINGLE JSON object on one line. No prose, no markdown fences. Required schema:

{"verdict":"replan"|"abandon"|"split_milestone"|"continue","reason":"...","replan_milestone":"<uuid-or-null>","sub_plans":[{"name":"...","description":"...","parallel_tasks":false}]}

- "reason" must be a one-sentence explanation grounded in the failure evidence.
- "replan_milestone" is the target milestone id for "replan". Omit or set null to re-plan the source milestone. For other verdicts it is ignored.
- "sub_plans" is required for "split_milestone" (at least 1 entry; chain runs sequentially in array order). Ignored for other verdicts. Empty sub_plans on a split verdict is treated as "continue".
- Never include text outside the JSON object."#;

/// System prompt for the tool-aware mode. Adds a short investigation
/// preamble before the same verdict schema: the model may issue a
/// bounded number of read-only tool calls to inspect the codebase
/// before deciding. Verdict schema and "no prose outside JSON" rule
/// are unchanged so the parser stays the same.
const DIAGNOSTIC_SYSTEM_PROMPT_TOOL_AWARE: &str = r#"You are a senior software engineer diagnosing why a coding task or milestone is stuck.

You have READ-ONLY access to the codebase via the provided tools (e.g. `read_file`, `search`, `glob`, memory query). Use them to *investigate* the failure before deciding. Do not attempt to modify state — only read-only tools are exposed.

After at most a handful of tool calls, pick exactly one recovery action for the orchestrator:

- "replan"          → the milestone should be re-attempted, optionally pointing the scheduler at a *different* milestone id known to the orchestrator. Use when the failure looks recoverable with a fresh plan.
- "abandon"         → the milestone is unsalvageable in this run; the scheduler should halt this branch.
- "split_milestone" → the milestone is too large; break it into a sequential chain of smaller sub-milestones. The orchestrator will create each entry in `sub_plans` as a new milestone, rewire the DAG, and walk into the first child.
- "continue"        → no useful action; let the existing failure path proceed.

When you are ready to commit to an action, respond with a SINGLE JSON object on one line and NO tool calls. No prose, no markdown fences. Required schema:

{"verdict":"replan"|"abandon"|"split_milestone"|"continue","reason":"...","replan_milestone":"<uuid-or-null>","sub_plans":[{"name":"...","description":"...","parallel_tasks":false}]}

- "reason" must be a one-sentence explanation grounded in evidence you actually gathered.
- "replan_milestone" is the target milestone id for "replan". Omit or set null to re-plan the source milestone. For other verdicts it is ignored.
- "sub_plans" is required for "split_milestone" (at least 1 entry; chain runs sequentially in array order). Ignored for other verdicts. Empty sub_plans on a split verdict is treated as "continue".
- Never include text outside the JSON object on the verdict turn."#;

/// Wire-format the LLM emits. Decoded into [`Handoff`] by
/// [`LlmDiagnosticResearcher::parse_response`]; kept private so the
/// schema is owned by this module and the public API surface is the
/// [`Handoff`] enum.
#[derive(Debug, Deserialize)]
struct LlmDiagnosticVerdict {
    verdict: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    replan_milestone: Option<String>,
    /// Sub-plans for the `split_milestone` verdict. Each entry becomes a
    /// child milestone via [`Orchestrator::split_milestone`]. Ignored
    /// for other verdicts. Empty / missing on a `split_milestone`
    /// verdict downgrades to [`Handoff::Continue`] at parse time so
    /// the orchestrator never tries to split into zero children.
    #[serde(default)]
    sub_plans: Vec<LlmSubPlan>,
}

/// On-wire shape of one entry in `LlmDiagnosticVerdict.sub_plans`.
/// Decoupled from [`MilestonePlan`] so we can be lenient about
/// missing fields without leaking the wire defaults into the public
/// API.
#[derive(Debug, Deserialize)]
struct LlmSubPlan {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    parallel_tasks: Option<bool>,
}

impl LlmSubPlan {
    fn into_plan(self) -> MilestonePlan {
        MilestonePlan {
            name: self.name,
            description: self.description,
            parallel_tasks: self.parallel_tasks.unwrap_or(false),
        }
    }
}

/// Real LLM-backed diagnostic. Builds a prompt from
/// [`DiagnosticContext`], calls the configured backend, and parses the
/// JSON verdict into a [`Handoff`]. Failure modes are *always* safe:
/// an LLM error, an unparseable response, or an unknown verdict all
/// fall back to [`Handoff::Continue`] (logged as a warning) — a stuck
/// diagnostic must never make the orchestrator's failure path worse
/// than not running the diagnostic at all.
///
/// Reuses the project's standard [`LlmBackend`] trait — no separate
/// model registry, no parallel retry stack. Callers point the
/// researcher at whatever backend (`llm.researcher` setting in the
/// Master spec) they prefer.
pub struct LlmDiagnosticResearcher {
    orchestrator: Weak<Orchestrator>,
    llm: Arc<dyn LlmBackend>,
    max_tokens: u64,
    temperature: f32,
    /// Optional read-only tool registry. When `Some`, `diagnose` runs a
    /// bounded tool-call loop so the model can `read_file` / `search`
    /// before deciding. When `None`, the diagnostic is single-shot
    /// (one LLM call against the inline failure summary).
    tool_registry: Option<Arc<ToolRegistry>>,
    /// Cap on tool-call rounds per invocation. Ignored when
    /// `tool_registry` is `None`. Defaults to
    /// [`DEFAULT_LLM_RESEARCHER_MAX_TOOL_ITERATIONS`].
    max_tool_iterations: usize,
}

impl LlmDiagnosticResearcher {
    pub fn new(orchestrator: Weak<Orchestrator>, llm: Arc<dyn LlmBackend>) -> Self {
        Self {
            orchestrator,
            llm,
            max_tokens: DEFAULT_LLM_RESEARCHER_MAX_TOKENS,
            temperature: DEFAULT_LLM_RESEARCHER_TEMPERATURE,
            tool_registry: None,
            max_tool_iterations: DEFAULT_LLM_RESEARCHER_MAX_TOOL_ITERATIONS,
        }
    }

    /// Builder-style cap override. Useful for cheaper / faster diagnostic
    /// models that don't need the default budget.
    pub fn with_max_tokens(mut self, max_tokens: u64) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = temperature;
        self
    }

    /// Wire a read-only tool registry so the diagnostic can inspect
    /// the codebase before deciding. The trigger filters by
    /// [`ToolBehavioralHints::read_only_hint`] at fire time — any tool
    /// whose hint is `false` is silently excluded from the definitions
    /// the model sees AND blocked if the model tries to call it by
    /// name anyway.
    ///
    /// `max_tool_iterations` caps how many tool rounds one diagnose
    /// call can run; pass `None` to keep the default.
    pub fn with_tools(
        mut self,
        registry: Arc<ToolRegistry>,
        max_tool_iterations: Option<usize>,
    ) -> Self {
        self.tool_registry = Some(registry);
        if let Some(cap) = max_tool_iterations {
            self.max_tool_iterations = cap;
        }
        self
    }

    /// Build the user-turn prompt from the diagnostic context. Pure
    /// function; uses the live orchestrator only via the `ctx` payload
    /// the trigger already resolved (avoids a second async lookup
    /// here). Public for tests that want to pin the prompt shape.
    pub fn build_user_prompt(ctx: &DiagnosticContext) -> String {
        let stage_label = match ctx.stage {
            Stage::OnMilestoneBlocked => "milestone blocked",
            Stage::OnTaskFailed => "task failed",
            other => return format!("Diagnostic fired for unsupported stage {other:?}"),
        };

        let mut out = String::new();
        out.push_str(&format!(
            "Stage: {stage_label}\nMilestone: {}\n",
            ctx.milestone
        ));
        if let Some(project) = ctx.project {
            out.push_str(&format!("Project: {project}\n"));
        }
        out.push_str(&format!(
            "Prior researcher invocations on this milestone: {}\n",
            ctx.prior_invocations
        ));
        if ctx.escalation {
            out.push_str(
                "Escalation source: loop detector — the generator's tool loop repeated itself \
                 enough to trip the doom-loop detector. Treat this as evidence the current \
                 plan is stuck; prefer `replan` or `split_milestone` over `continue`.\n",
            );
        }
        if !ctx.reason.is_empty() {
            out.push_str(&format!("Failure reason: {}\n", ctx.reason));
        }

        if let Some(task) = &ctx.failing_task {
            out.push_str("\nFailing task:\n");
            out.push_str(&format!("- id: {}\n", task.id));
            out.push_str(&format!("- description: {}\n", task.description));
            if task.spawn_depth > 0 {
                out.push_str(&format!(
                    "- spawn_depth: {} (failure-derived child)\n",
                    task.spawn_depth
                ));
            }
            if let Some(parent) = task.parent {
                out.push_str(&format!("- parent: {parent}\n"));
            }
            if let Some(validation) = &task.validation_result {
                if !validation.errors.is_empty() {
                    out.push_str("- validator errors:\n");
                    for (i, err) in validation.errors.iter().take(5).enumerate() {
                        out.push_str(&format!("  {}. {err}\n", i + 1));
                    }
                    if validation.errors.len() > 5 {
                        out.push_str(&format!("  ... and {} more\n", validation.errors.len() - 5));
                    }
                }
            }
            if let Some(rejected) = &task.rejected_reason {
                out.push_str(&format!("- rejected_reason: {rejected}\n"));
            }
        }

        out.push_str(
            "\nReturn exactly one JSON object per the schema in your system prompt. No prose.",
        );
        out
    }

    /// Parse the raw LLM response into a [`Handoff`]. Tolerant: strips
    /// markdown fences if the model added them, falls back to
    /// `Handoff::Continue` on any parse failure or unknown verdict.
    /// `source_milestone` is the milestone the trigger is reasoning
    /// about — used as the default target when the verdict is "replan"
    /// and the model didn't specify one.
    pub fn parse_response(raw: &str, source_milestone: MilestoneId) -> Handoff {
        let trimmed = strip_optional_fences(raw.trim());
        let verdict: LlmDiagnosticVerdict = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    error = %e,
                    raw = %raw,
                    "researcher: LLM response was not valid JSON; defaulting to Continue"
                );
                return Handoff::Continue;
            }
        };

        match verdict.verdict.as_str() {
            "replan" => {
                let target = verdict
                    .replan_milestone
                    .as_deref()
                    .and_then(|raw| match raw.parse::<MilestoneId>() {
                        Ok(id) => Some(id),
                        Err(e) => {
                            warn!(
                                raw = %raw,
                                error = %e,
                                "researcher: replan_milestone unparseable; defaulting to source"
                            );
                            None
                        }
                    })
                    .unwrap_or(source_milestone);
                Handoff::Replan {
                    milestone: target,
                    reason: verdict.reason,
                }
            }
            "abandon" => Handoff::Abandon {
                milestone: source_milestone,
                reason: verdict.reason,
            },
            "split_milestone" => {
                if verdict.sub_plans.is_empty() {
                    warn!(
                        milestone = %source_milestone,
                        "researcher: split_milestone verdict carried no sub_plans; defaulting to Continue"
                    );
                    Handoff::Continue
                } else {
                    let into: Vec<MilestonePlan> = verdict
                        .sub_plans
                        .into_iter()
                        .map(LlmSubPlan::into_plan)
                        .collect();
                    Handoff::SplitMilestone {
                        milestone: source_milestone,
                        into,
                    }
                }
            }
            "continue" => Handoff::Continue,
            other => {
                warn!(
                    verdict = %other,
                    "researcher: unknown verdict from LLM; defaulting to Continue"
                );
                Handoff::Continue
            }
        }
    }
}

fn strip_optional_fences(s: &str) -> &str {
    // The model is *told* not to use markdown fences, but real models
    // sometimes ignore that. Strip a single leading ```json (or ```)
    // and trailing ``` if present.
    let s = s.trim();
    let s = s
        .strip_prefix("```json")
        .or_else(|| s.strip_prefix("```"))
        .unwrap_or(s);
    s.strip_suffix("```").unwrap_or(s).trim()
}

impl LlmDiagnosticResearcher {
    /// Build the initial conversation. Splits out so the tool-loop and
    /// single-shot paths share the same prompt construction.
    fn initial_messages(&self, ctx: &DiagnosticContext) -> Vec<LlmMessage> {
        let system = if self.tool_registry.is_some() {
            DIAGNOSTIC_SYSTEM_PROMPT_TOOL_AWARE
        } else {
            DIAGNOSTIC_SYSTEM_PROMPT
        };
        vec![
            LlmMessage {
                role: LlmRole::System,
                content: system.to_string(),
                ..Default::default()
            },
            LlmMessage {
                role: LlmRole::User,
                content: Self::build_user_prompt(ctx),
                ..Default::default()
            },
        ]
    }

    fn llm_config(&self) -> LlmConfig {
        LlmConfig {
            temperature: self.temperature,
            max_tokens: self.max_tokens,
            ..Default::default()
        }
    }

    /// Single-shot diagnose path. Used when no tool registry is wired.
    async fn diagnose_single_shot(&self, ctx: &DiagnosticContext) -> Handoff {
        let response = match self
            .llm
            .chat(self.initial_messages(ctx), None, self.llm_config())
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!(
                    error = %e,
                    milestone = %ctx.milestone,
                    "researcher: LLM chat failed; defaulting to Continue"
                );
                return Handoff::Continue;
            }
        };
        Self::parse_response(&response.content, ctx.milestone)
    }

    /// Resolve read-only tool definitions from the wired registry. Any
    /// tool whose [`ToolBehavioralHints::read_only_hint`] is `false`
    /// is excluded. Returns the definitions to ship to the LLM *and*
    /// the set of allowed tool names — the latter is the safety check
    /// at execution time (the model can hallucinate any name; we only
    /// invoke names that survived the read-only filter).
    async fn read_only_tool_defs(
        registry: &ToolRegistry,
    ) -> (Vec<LlmToolDefinition>, std::collections::HashSet<String>) {
        let mut defs = Vec::new();
        let mut allowed = std::collections::HashSet::new();
        let names = registry.list_names().await;
        for name in names {
            let Some(tool) = registry.get(&name).await else {
                continue;
            };
            let hints = tool.behavioral_hints();
            if !hints.read_only_hint || hints.destructive_hint {
                continue;
            }
            defs.push(LlmToolDefinition {
                name: tool.name().to_string(),
                description: tool.description(),
                input_schema: tool.input_schema(),
            });
            allowed.insert(tool.name().to_string());
        }
        (defs, allowed)
    }

    /// Tool-loop diagnose path. Bounded by `self.max_tool_iterations`;
    /// every failure mode (LLM error, blocked tool, tool execution
    /// failure, iteration cap exhausted, unparseable verdict) degrades
    /// to `Handoff::Continue` with a warn log.
    async fn diagnose_with_tools(
        &self,
        ctx: &DiagnosticContext,
        registry: &ToolRegistry,
    ) -> Handoff {
        let (tool_defs, allowed_names) = Self::read_only_tool_defs(registry).await;
        let mut conversation = self.initial_messages(ctx);

        for iteration in 0..=self.max_tool_iterations {
            // `tools = None` instead of `Some(empty)` keeps the request
            // shape identical to the single-shot path when no
            // read-only tools resolved.
            let tools = if tool_defs.is_empty() {
                None
            } else {
                Some(tool_defs.clone())
            };

            let response = match self
                .llm
                .chat(conversation.clone(), tools, self.llm_config())
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    warn!(
                        error = %e,
                        milestone = %ctx.milestone,
                        iteration,
                        "researcher (tools): LLM chat failed; defaulting to Continue"
                    );
                    return Handoff::Continue;
                }
            };

            let tool_calls = response.tool_calls.clone().unwrap_or_default();
            if tool_calls.is_empty() {
                // Verdict turn. Parse and return.
                return Self::parse_response(&response.content, ctx.milestone);
            }

            if iteration == self.max_tool_iterations {
                // Model is still asking for tools after the budget.
                // Try parsing whatever text it produced anyway; on
                // empty/invalid, default to Continue.
                warn!(
                    milestone = %ctx.milestone,
                    cap = self.max_tool_iterations,
                    "researcher (tools): tool-iteration cap reached; forcing a verdict"
                );
                return Self::parse_response(&response.content, ctx.milestone);
            }

            // Echo the assistant turn including its thinking blocks +
            // tool_calls, matching the Generator pattern. Providers
            // that bind reasoning signatures to subsequent tool_result
            // turns (MiniMax) require this round-trip.
            conversation.push(LlmMessage {
                role: LlmRole::Assistant,
                content: response.content.clone(),
                tool_calls: Some(tool_calls.clone()),
                thinking_blocks: response.thinking_blocks.clone(),
                ..Default::default()
            });

            for call in tool_calls {
                let observation = self
                    .run_one_tool_call(registry, &allowed_names, &call)
                    .await;
                conversation.push(LlmMessage {
                    role: LlmRole::User,
                    content: observation.content,
                    tool_call_id: Some(call.id.clone()),
                    tool_result_is_error: observation.is_error,
                    ..Default::default()
                });
            }
        }

        // Loop fell out without a verdict (defensive — the for-loop
        // body always returns or appends). Safe default.
        Handoff::Continue
    }

    /// Execute one tool call and shape it as a `tool_result` user
    /// turn observation. Never panics; tool errors / blocked tools /
    /// missing tools are surfaced as `is_error: true` so the model
    /// can react. Blocking non-allowed tools here is the safety net
    /// — even if the model fabricates a tool name we never claimed
    /// to support, we won't invoke it.
    async fn run_one_tool_call(
        &self,
        registry: &ToolRegistry,
        allowed: &std::collections::HashSet<String>,
        call: &LlmToolCall,
    ) -> ToolObservation {
        if !allowed.contains(&call.name) {
            warn!(
                tool = %call.name,
                "researcher (tools): blocked non-allowlisted tool call"
            );
            return ToolObservation {
                content: format!(
                    "Tool '{}' is not available in read-only diagnostic mode.",
                    call.name
                ),
                is_error: true,
            };
        }

        let Some(tool) = registry.get(&call.name).await else {
            return ToolObservation {
                content: format!("Tool '{}' not found in registry.", call.name),
                is_error: true,
            };
        };

        // Re-check read_only_hint at execution time. The allowlist set
        // was built from a snapshot of the registry; tools could
        // theoretically be replaced between the snapshot and now.
        let hints = tool.behavioral_hints();
        if !hints.read_only_hint || hints.destructive_hint {
            warn!(
                tool = %call.name,
                "researcher (tools): tool no longer read-only at exec time"
            );
            return ToolObservation {
                content: format!(
                    "Tool '{}' is not safe for read-only diagnostic execution.",
                    call.name
                ),
                is_error: true,
            };
        }

        match tool.execute(call.arguments.clone()).await {
            Ok(output) => ToolObservation::from(output),
            Err(e) => ToolObservation {
                content: format!("Tool '{}' execution failed: {e}", call.name),
                is_error: true,
            },
        }
    }
}

/// Internal observation row produced for each tool call. Maps to the
/// user-turn `tool_result` shape downstream.
struct ToolObservation {
    content: String,
    is_error: bool,
}

impl From<ToolOutput> for ToolObservation {
    fn from(output: ToolOutput) -> Self {
        let mut parts = Vec::with_capacity(output.content.len());
        for piece in &output.content {
            match piece {
                ToolResultContent::Text(t) => parts.push(t.clone()),
                ToolResultContent::Json(v) => parts.push(v.to_string()),
                ToolResultContent::Error(e) => parts.push(format!("error: {e}")),
                ToolResultContent::Image { media_type, .. } => {
                    parts.push(format!("[image:{media_type}]"))
                }
            }
        }
        let content = if parts.is_empty() {
            String::from("(empty tool output)")
        } else {
            parts.join("\n")
        };
        ToolObservation {
            content,
            is_error: output.is_error,
        }
    }
}

#[async_trait]
impl DiagnosticResearcher for LlmDiagnosticResearcher {
    async fn diagnose(&self, ctx: &DiagnosticContext) -> Handoff {
        // Best-effort orchestrator upgrade; not strictly required for
        // the LLM call itself but lets the diagnostic look up extra
        // context in future revisions without re-plumbing.
        let _ = self.orchestrator.upgrade();

        match &self.tool_registry {
            Some(registry) => self.diagnose_with_tools(ctx, registry).await,
            None => self.diagnose_single_shot(ctx).await,
        }
    }
}

/// Per-milestone invocation counter shared across all fires of a
/// single `ResearcherTrigger`. `Arc<Mutex<HashMap>>` is enough for the
/// expected fanout (milestones per project, not thousands).
#[derive(Debug)]
pub struct ResearcherBudget {
    cap: usize,
    counts: Mutex<HashMap<MilestoneId, usize>>,
}

impl ResearcherBudget {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            counts: Mutex::new(HashMap::new()),
        }
    }

    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Returns the current count for `milestone` without mutating.
    pub fn current(&self, milestone: MilestoneId) -> usize {
        self.counts
            .lock()
            .map(|g| g.get(&milestone).copied().unwrap_or(0))
            .unwrap_or(0)
    }

    /// Returns `true` if `milestone` has reached the cap and further
    /// invocations should be auto-halted.
    pub fn exceeded(&self, milestone: MilestoneId) -> bool {
        self.current(milestone) >= self.cap
    }

    /// Records one invocation against `milestone`. Returns the new
    /// count.
    pub fn record(&self, milestone: MilestoneId) -> usize {
        let mut guard = self.counts.lock().expect("budget map poisoned");
        let entry = guard.entry(milestone).or_insert(0);
        *entry += 1;
        *entry
    }
}

/// `OnMilestoneBlocked` / `OnTaskFailed` trigger that invokes a
/// diagnostic researcher.
pub struct ResearcherTrigger {
    stages: &'static [Stage],
    orchestrator: Weak<Orchestrator>,
    diagnostic: Arc<dyn DiagnosticResearcher>,
    budget: Arc<ResearcherBudget>,
}

impl ResearcherTrigger {
    pub fn new(
        stages: &'static [Stage],
        orchestrator: Weak<Orchestrator>,
        diagnostic: Arc<dyn DiagnosticResearcher>,
        budget: Arc<ResearcherBudget>,
    ) -> Self {
        Self {
            stages,
            orchestrator,
            diagnostic,
            budget,
        }
    }

    /// Convenience constructor for the default
    /// `OnMilestoneBlocked` + `OnTaskFailed` wiring.
    pub fn with_default_stages(
        orchestrator: Weak<Orchestrator>,
        diagnostic: Arc<dyn DiagnosticResearcher>,
        budget: Arc<ResearcherBudget>,
    ) -> Self {
        Self::new(
            &[Stage::OnMilestoneBlocked, Stage::OnTaskFailed],
            orchestrator,
            diagnostic,
            budget,
        )
    }

    /// Resolve the milestone the trigger should reason about.
    ///
    /// For `OnMilestoneBlocked` the milestone is on the context. For
    /// `OnTaskFailed` we look up the failing task and use its
    /// `task.milestone` (loose tasks have none and the trigger skips).
    async fn resolve_milestone(
        &self,
        ctx: &TriggerContext,
        orch: &Arc<Orchestrator>,
    ) -> Option<(MilestoneId, Option<ProjectId>, Option<Task>)> {
        if let Some(milestone) = ctx.milestone {
            return Some((milestone, ctx.project, None));
        }
        let task_id: TaskId = ctx.task?;
        let task = match orch.get_task(task_id).await {
            Ok(t) => t,
            Err(e) => {
                warn!(
                    task_id = %task_id,
                    error = %e,
                    "researcher: failing task not found; skipping"
                );
                return None;
            }
        };
        let milestone = task.milestone?;
        // Best-effort project lookup; not required for the diagnostic
        // to do its work.
        let project = orch.get_milestone(milestone).await.ok().map(|m| m.project);
        Some((milestone, project, Some(task)))
    }

    /// Translate a [`Handoff`] verdict into a [`TriggerOutcome`] for
    /// the non-async verdicts (`Replan` / `Abandon` / `Continue`). The
    /// async `SplitMilestone` path goes through [`Self::apply`], which
    /// calls into [`Orchestrator::split_milestone`].
    ///
    /// Public + standalone so unit tests can pin the simple translator
    /// branches without going through the trigger.
    pub fn translate(handoff: Handoff) -> TriggerOutcome {
        match handoff {
            Handoff::Replan { milestone, reason } => {
                info!(milestone = %milestone, reason = %reason, "researcher: replan → reroute");
                TriggerOutcome::Reroute(milestone)
            }
            Handoff::Abandon { milestone, reason } => {
                warn!(milestone = %milestone, reason = %reason, "researcher: abandon → halt");
                TriggerOutcome::Halt(reason)
            }
            Handoff::SplitMilestone { milestone, into } => {
                // `translate` is sync and can't call the orchestrator.
                // Production code goes through `apply` instead; we keep
                // this arm so legacy unit tests still compile and the
                // sync surface remains a safe fallback (Continue rather
                // than a panic). See `apply` for the live behavior.
                info!(
                    milestone = %milestone,
                    sub_plans = into.len(),
                    "researcher: translate() called on SplitMilestone; production path is apply()"
                );
                TriggerOutcome::Continue
            }
            Handoff::Continue => TriggerOutcome::Continue,
        }
    }

    /// Apply a [`Handoff`] verdict against the live orchestrator. For
    /// `SplitMilestone` this creates the child milestones, rewires
    /// downstream DAG edges, and reroutes the scheduler into the
    /// first child. Other verdicts delegate to [`Self::translate`].
    ///
    /// Errors from `Orchestrator::split_milestone` (e.g. empty
    /// sub_plans, missing source) degrade to [`TriggerOutcome::Continue`]
    /// with a warning — a broken diagnostic must not make recovery
    /// worse than skipping it.
    pub async fn apply(&self, handoff: Handoff, orch: &Arc<Orchestrator>) -> TriggerOutcome {
        match handoff {
            Handoff::SplitMilestone { milestone, into } => {
                let sub_plan_count = into.len();
                match orch.split_milestone(milestone, into).await {
                    Ok(children) => {
                        let first = match children.first() {
                            Some(id) => *id,
                            None => {
                                warn!(
                                    milestone = %milestone,
                                    "researcher: split_milestone returned no children; continuing"
                                );
                                return TriggerOutcome::Continue;
                            }
                        };
                        info!(
                            source = %milestone,
                            children = children.len(),
                            first_child = %first,
                            "researcher: split → reroute to first child"
                        );
                        TriggerOutcome::Reroute(first)
                    }
                    Err(e) => {
                        warn!(
                            milestone = %milestone,
                            sub_plans = sub_plan_count,
                            error = %e,
                            "researcher: split_milestone failed; defaulting to Continue"
                        );
                        TriggerOutcome::Continue
                    }
                }
            }
            other => Self::translate(other),
        }
    }
}

#[async_trait]
impl Trigger for ResearcherTrigger {
    fn name(&self) -> &'static str {
        "researcher"
    }

    fn stages(&self) -> &'static [Stage] {
        self.stages
    }

    async fn run(&self, ctx: &TriggerContext) -> TriggerOutcome {
        let Some(orch) = self.orchestrator.upgrade() else {
            warn!("researcher: orchestrator dropped; skipping");
            return TriggerOutcome::Continue;
        };

        let Some((milestone, project, failing_task)) = self.resolve_milestone(ctx, &orch).await
        else {
            // No milestone to reason about — e.g. OnTaskFailed for a
            // loose task. Don't invoke the diagnostic.
            info!(
                stage = ?ctx.stage,
                "researcher: no milestone resolved; skipping diagnostic"
            );
            return TriggerOutcome::Continue;
        };

        if self.budget.exceeded(milestone) {
            let reason = format!(
                "researcher budget exceeded for milestone {milestone}: {}/{} invocations",
                self.budget.current(milestone),
                self.budget.cap()
            );
            warn!(milestone = %milestone, "{}", reason);
            return TriggerOutcome::Halt(reason);
        }

        let diag_ctx = DiagnosticContext {
            stage: ctx.stage,
            project,
            milestone,
            failing_task,
            reason: if ctx.escalation {
                "loop detector escalation".to_string()
            } else {
                String::new()
            },
            prior_invocations: self.budget.current(milestone),
            escalation: ctx.escalation,
        };

        let handoff = self.diagnostic.diagnose(&diag_ctx).await;
        let new_count = self.budget.record(milestone);
        info!(
            milestone = %milestone,
            invocations = new_count,
            cap = self.budget.cap(),
            handoff = ?handoff,
            "researcher: diagnosed"
        );
        self.apply(handoff, &orch).await
    }
}

/// Optional config blob shape for the `researcher` trigger entry in
/// `.swell/triggers.json`. All fields optional; unknown keys ignored
/// (forward-compat).
#[derive(Debug, Default, Deserialize)]
struct ResearcherConfig {
    #[serde(default)]
    max_invocations: Option<usize>,
    /// Diagnostic backing: `"stub"` (default — returns `Continue`,
    /// preserves existing behavior) or `"live"` (calls the
    /// `LlmDiagnosticResearcher` wired into the factory). Only
    /// honored by [`register_mode_switched_researcher_factory`]; the
    /// older single-purpose factories ignore it.
    #[serde(default)]
    mode: Option<String>,
    /// When `mode = "live"`, plumb the shared `ToolRegistry` into
    /// the diagnostic so it can `read_file` / `search` before
    /// deciding. Default: `true` when a tool registry is available,
    /// `false` otherwise. Ignored in stub mode.
    #[serde(default)]
    use_tools: Option<bool>,
    /// Cap on tool-call rounds per live diagnostic invocation when
    /// `use_tools` is on. Default
    /// [`DEFAULT_LLM_RESEARCHER_MAX_TOOL_ITERATIONS`].
    #[serde(default)]
    max_tool_iterations: Option<usize>,
}

/// Which diagnostic backing the mode-switched factory should build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResearcherMode {
    Stub,
    Live,
}

impl ResearcherMode {
    fn from_config(raw: Option<&str>) -> Self {
        match raw.map(|s| s.to_ascii_lowercase()) {
            None => ResearcherMode::Stub,
            Some(ref s) if s == "stub" => ResearcherMode::Stub,
            Some(ref s) if s == "live" => ResearcherMode::Live,
            Some(other) => {
                warn!(
                    raw = %other,
                    "researcher: unknown mode in config; defaulting to stub"
                );
                ResearcherMode::Stub
            }
        }
    }
}

/// Register the `researcher` factory on `factories`. The caller
/// provides a `diagnostic_provider` closure invoked once per trigger
/// instance (per `.swell/triggers.json` entry) to supply the
/// diagnostic. Daemons that don't have a real LLM diagnostic should
/// hand in [`StubDiagnosticResearcher`] — the trigger then returns
/// `Continue` on every fire, preserving the default-on-without-
/// behavior-change contract.
///
/// Config blob accepts `{"max_invocations": <usize>}` to override
/// [`DEFAULT_RESEARCHER_INVOCATION_CAP`].
pub fn register_researcher_factory<P>(
    factories: &mut TriggerFactoryRegistry,
    orchestrator: Weak<Orchestrator>,
    diagnostic_provider: P,
) where
    P: Fn() -> Arc<dyn DiagnosticResearcher> + Send + Sync + 'static,
{
    let provider = Arc::new(diagnostic_provider);
    factories.register("researcher", move |stages, config| {
        let leaked: &'static [Stage] = Box::leak(stages.to_vec().into_boxed_slice());
        let cfg: ResearcherConfig = serde_json::from_value(config.clone()).unwrap_or_default();
        let cap = cfg
            .max_invocations
            .unwrap_or(DEFAULT_RESEARCHER_INVOCATION_CAP);
        let budget = Arc::new(ResearcherBudget::new(cap));
        let diagnostic = provider();
        Some(Arc::new(ResearcherTrigger::new(
            leaked,
            orchestrator.clone(),
            diagnostic,
            budget,
        )) as Arc<dyn Trigger>)
    });
}

/// Convenience: register the `researcher` factory with a
/// [`StubDiagnosticResearcher`] backing — the default daemon wiring.
pub fn register_default_researcher_factory(
    factories: &mut TriggerFactoryRegistry,
    orchestrator: Weak<Orchestrator>,
) {
    register_researcher_factory(factories, orchestrator, || {
        Arc::new(StubDiagnosticResearcher::new()) as Arc<dyn DiagnosticResearcher>
    });
}

/// Convenience: register the `researcher` factory with a
/// [`LlmDiagnosticResearcher`] backed by `llm`. Each trigger built
/// from this factory clones the same `Arc<dyn LlmBackend>`. Production
/// daemons opt into the live diagnostic by calling this instead of
/// [`register_default_researcher_factory`].
///
/// The orchestrator weak ref is captured once and shared across all
/// trigger instances the factory hands out — the diagnostic itself
/// only needs `Weak<Orchestrator>` for forward-compat context lookups,
/// the budget enforcement and milestone resolution live on the trigger.
pub fn register_llm_researcher_factory(
    factories: &mut TriggerFactoryRegistry,
    orchestrator: Weak<Orchestrator>,
    llm: Arc<dyn LlmBackend>,
) {
    register_researcher_factory(factories, orchestrator.clone(), move || {
        Arc::new(LlmDiagnosticResearcher::new(
            orchestrator.clone(),
            Arc::clone(&llm),
        )) as Arc<dyn DiagnosticResearcher>
    });
}

/// Register the `researcher` factory with config-driven mode selection.
///
/// This is the *production* daemon wiring: the operator picks `stub`
/// or `live` via `.swell/triggers.json`, and the factory plumbs the
/// shared LLM + tool registry through to the diagnostic only when
/// `mode = "live"`. Defaults match the legacy behavior — absence of
/// `mode` resolves to `stub`, so existing setups continue with no
/// behavior change.
///
/// Recognized config keys (all optional):
///
/// - `mode`: `"stub"` (default) or `"live"`. Unknown values warn and
///   fall back to `stub`.
/// - `max_invocations`: `usize` — researcher budget cap per milestone.
/// - `use_tools`: `bool` — when `mode = "live"`, plumb the shared
///   `ToolRegistry` into the diagnostic so it can issue read-only
///   tool calls. Defaults to `true` when `tool_registry` is `Some`,
///   `false` otherwise.
/// - `max_tool_iterations`: `usize` — cap on tool rounds per
///   diagnostic invocation when `use_tools` is on.
///
/// `tool_registry` may be `None`: the factory still works in stub
/// mode, and live mode falls back to single-shot (with a warning)
/// when `use_tools` is on but no registry was supplied.
pub fn register_mode_switched_researcher_factory(
    factories: &mut TriggerFactoryRegistry,
    orchestrator: Weak<Orchestrator>,
    llm: Arc<dyn LlmBackend>,
    tool_registry: Option<Arc<ToolRegistry>>,
) {
    factories.register("researcher", move |stages, config| {
        let leaked: &'static [Stage] = Box::leak(stages.to_vec().into_boxed_slice());
        let cfg: ResearcherConfig = serde_json::from_value(config.clone()).unwrap_or_default();
        let cap = cfg
            .max_invocations
            .unwrap_or(DEFAULT_RESEARCHER_INVOCATION_CAP);
        let budget = Arc::new(ResearcherBudget::new(cap));
        let mode = ResearcherMode::from_config(cfg.mode.as_deref());

        let diagnostic: Arc<dyn DiagnosticResearcher> = match mode {
            ResearcherMode::Stub => Arc::new(StubDiagnosticResearcher::new()),
            ResearcherMode::Live => {
                let mut diag =
                    LlmDiagnosticResearcher::new(orchestrator.clone(), Arc::clone(&llm));
                // `use_tools` default: true if we have a registry,
                // false if not. Operators who want to force-disable
                // tool access on a live diagnostic set
                // `use_tools: false` explicitly.
                let want_tools = cfg.use_tools.unwrap_or_else(|| tool_registry.is_some());
                if want_tools {
                    match &tool_registry {
                        Some(registry) => {
                            diag = diag.with_tools(
                                Arc::clone(registry),
                                cfg.max_tool_iterations,
                            );
                        }
                        None => {
                            warn!(
                                "researcher: live mode requested tools but no ToolRegistry was supplied; running single-shot"
                            );
                        }
                    }
                }
                Arc::new(diag)
            }
        };

        info!(
            mode = ?mode,
            budget_cap = cap,
            "researcher: factory built trigger from config"
        );
        Some(Arc::new(ResearcherTrigger::new(
            leaked,
            orchestrator.clone(),
            diagnostic,
            budget,
        )) as Arc<dyn Trigger>)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::triggers::TriggerContext;
    use crate::OrchestratorBuilder;
    use swell_core::{Goal, MilestoneStatus};

    fn make_orchestrator() -> Arc<Orchestrator> {
        OrchestratorBuilder::new().build()
    }

    fn task_failed_ctx(task: TaskId) -> TriggerContext {
        TriggerContext::for_task(Stage::OnTaskFailed, task)
    }

    fn milestone_blocked_ctx(project: ProjectId, milestone: MilestoneId) -> TriggerContext {
        TriggerContext::for_milestone(Stage::OnMilestoneBlocked, project, milestone)
    }

    /// `Replan` should translate to `Reroute(milestone)` straight through.
    #[test]
    fn translate_replan_to_reroute() {
        let m = MilestoneId::new();
        let outcome = ResearcherTrigger::translate(Handoff::Replan {
            milestone: m,
            reason: "stuck on validator".into(),
        });
        assert_eq!(outcome, TriggerOutcome::Reroute(m));
    }

    /// `Abandon` should translate to `Halt(reason)`.
    #[test]
    fn translate_abandon_to_halt() {
        let m = MilestoneId::new();
        let outcome = ResearcherTrigger::translate(Handoff::Abandon {
            milestone: m,
            reason: "out of options".into(),
        });
        match outcome {
            TriggerOutcome::Halt(r) => assert_eq!(r, "out of options"),
            other => panic!("expected Halt, got {other:?}"),
        }
    }

    /// `Continue` is identity.
    #[test]
    fn translate_continue_to_continue() {
        let outcome = ResearcherTrigger::translate(Handoff::Continue);
        assert_eq!(outcome, TriggerOutcome::Continue);
    }

    /// `translate` is sync and intentionally returns `Continue` for
    /// `SplitMilestone` — the live path goes through `apply`. Pin this
    /// so refactoring doesn't accidentally invoke an async path from
    /// the sync translator and miss the orchestrator side effects.
    #[test]
    fn translate_split_milestone_is_continue_sync_fallback() {
        let m = MilestoneId::new();
        let outcome = ResearcherTrigger::translate(Handoff::SplitMilestone {
            milestone: m,
            into: vec![MilestonePlan {
                name: "narrow".into(),
                description: None,
                parallel_tasks: false,
            }],
        });
        assert_eq!(outcome, TriggerOutcome::Continue);
    }

    /// `apply` (the async production path) on `SplitMilestone` creates
    /// the child milestones via `Orchestrator::split_milestone` and
    /// returns `Reroute(first_child)`.
    #[tokio::test]
    async fn apply_split_milestone_creates_children_and_reroutes_to_first() {
        let orch = make_orchestrator();
        let project = orch
            .create_project(Goal::new("split apply", swell_core::TaskId::new()))
            .await;
        let source = orch
            .create_milestone(project.id, "wide".into())
            .await
            .unwrap();

        let diagnostic = Arc::new(StubDiagnosticResearcher::new());
        let budget = Arc::new(ResearcherBudget::new(2));
        let trigger =
            ResearcherTrigger::with_default_stages(Arc::downgrade(&orch), diagnostic, budget);

        let outcome = trigger
            .apply(
                Handoff::SplitMilestone {
                    milestone: source.id,
                    into: vec![
                        MilestonePlan {
                            name: "narrow-a".into(),
                            description: Some("lexer".into()),
                            parallel_tasks: false,
                        },
                        MilestonePlan {
                            name: "narrow-b".into(),
                            description: None,
                            parallel_tasks: true,
                        },
                    ],
                },
                &orch,
            )
            .await;

        // Verify the source is now Blocked, two children exist with the
        // expected names + parallel flag, and the chain is sequential.
        let project_milestones = orch.get_milestones_for_project(project.id).await.unwrap();
        let source_status = project_milestones
            .iter()
            .find(|m| m.id == source.id)
            .unwrap()
            .status;
        assert_eq!(
            source_status,
            MilestoneStatus::Blocked,
            "source milestone must be Blocked after split"
        );
        let children: Vec<_> = project_milestones
            .iter()
            .filter(|m| m.id != source.id)
            .collect();
        assert_eq!(children.len(), 2, "expected two child milestones");
        let by_name: std::collections::HashMap<_, _> =
            children.iter().map(|m| (m.title.as_str(), *m)).collect();
        let a = by_name["narrow-a"];
        let b = by_name["narrow-b"];
        assert!(!a.parallel_tasks);
        assert!(b.parallel_tasks);
        assert!(
            a.depends_on.is_empty(),
            "first child inherits source.depends_on (empty here)"
        );
        assert_eq!(
            b.depends_on,
            vec![a.id],
            "second child depends on first → sequential chain"
        );

        // And the trigger rerouted into the first child.
        match outcome {
            TriggerOutcome::Reroute(target) => assert_eq!(
                target, a.id,
                "apply must reroute into the FIRST child of the chain"
            ),
            other => panic!("expected Reroute(first_child), got {other:?}"),
        }
    }

    /// Downstream milestones that depended on the source get rewired to
    /// depend on the LAST child of the chain, so the rest of the DAG
    /// waits for the whole split to finish.
    #[tokio::test]
    async fn split_milestone_rewires_downstream_dependencies_to_chain_tail() {
        let orch = make_orchestrator();
        let project = orch
            .create_project(Goal::new("split rewire", swell_core::TaskId::new()))
            .await;
        let source = orch
            .create_milestone(project.id, "source".into())
            .await
            .unwrap();
        let downstream = orch
            .create_milestone(project.id, "downstream".into())
            .await
            .unwrap();
        // Make downstream depend on source.
        {
            let sm = orch.state_machine();
            let sm = sm.read().await;
            sm.with_milestone_mut(downstream.id, |m| {
                m.depends_on.push(source.id);
                Ok(())
            })
            .unwrap();
        }

        let children = orch
            .split_milestone(
                source.id,
                vec![
                    MilestonePlan {
                        name: "p1".into(),
                        description: None,
                        parallel_tasks: false,
                    },
                    MilestonePlan {
                        name: "p2".into(),
                        description: None,
                        parallel_tasks: false,
                    },
                ],
            )
            .await
            .unwrap();
        assert_eq!(children.len(), 2);
        let tail = *children.last().unwrap();

        let post = orch.get_milestone(downstream.id).await.unwrap();
        assert!(
            !post.depends_on.contains(&source.id),
            "edge on source must be rewritten"
        );
        assert!(
            post.depends_on.contains(&tail),
            "downstream must now depend on the chain tail; got {:?}",
            post.depends_on
        );
    }

    /// `Orchestrator::split_milestone` refuses an empty `sub_plans`
    /// argument — splitting into zero children is meaningless and would
    /// leave the scheduler stuck.
    #[tokio::test]
    async fn split_milestone_with_empty_sub_plans_errors() {
        let orch = make_orchestrator();
        let project = orch
            .create_project(Goal::new("empty split", swell_core::TaskId::new()))
            .await;
        let source = orch
            .create_milestone(project.id, "source".into())
            .await
            .unwrap();
        let err = orch.split_milestone(source.id, vec![]).await.unwrap_err();
        assert!(
            format!("{err}").contains("empty sub_plans"),
            "error must call out the empty-sub_plans condition; got: {err}"
        );
    }

    /// `apply` on `SplitMilestone` with an empty `sub_plans` (which
    /// should never come out of `parse_response` — it downgrades — but
    /// could come from a hand-built `Handoff`) safely degrades to
    /// `Continue` so the orchestrator's failure path isn't blocked by
    /// a buggy diagnostic constructor.
    #[tokio::test]
    async fn apply_split_milestone_with_empty_sub_plans_degrades_to_continue() {
        let orch = make_orchestrator();
        let project = orch
            .create_project(Goal::new("empty apply", swell_core::TaskId::new()))
            .await;
        let source = orch
            .create_milestone(project.id, "source".into())
            .await
            .unwrap();
        let diagnostic = Arc::new(StubDiagnosticResearcher::new());
        let budget = Arc::new(ResearcherBudget::new(2));
        let trigger =
            ResearcherTrigger::with_default_stages(Arc::downgrade(&orch), diagnostic, budget);
        let outcome = trigger
            .apply(
                Handoff::SplitMilestone {
                    milestone: source.id,
                    into: Vec::new(),
                },
                &orch,
            )
            .await;
        assert_eq!(outcome, TriggerOutcome::Continue);
        // Source must NOT have been Blocked (no children produced).
        let post = orch.get_milestone(source.id).await.unwrap();
        assert_eq!(
            post.status,
            MilestoneStatus::Pending,
            "failed split must not park the source"
        );
    }

    /// Budget enforces the cap: cap-th and later invocations short-
    /// circuit with `Halt("researcher budget exceeded")` *before*
    /// invoking the diagnostic.
    #[tokio::test]
    async fn budget_cap_halts_after_threshold() {
        let orch = make_orchestrator();
        let project = orch
            .create_project(Goal::new("budget smoke", swell_core::TaskId::new()))
            .await;
        let milestone = orch.create_milestone(project.id, "m".into()).await.unwrap();

        // Stub that returns Continue every time. We want to prove the
        // *trigger* halts independently of the diagnostic verdict.
        let diagnostic = Arc::new(StubDiagnosticResearcher::new());
        let budget = Arc::new(ResearcherBudget::new(2));
        let trigger = ResearcherTrigger::with_default_stages(
            Arc::downgrade(&orch),
            diagnostic.clone(),
            budget.clone(),
        );
        let ctx = milestone_blocked_ctx(project.id, milestone.id);

        // Two invocations are allowed; both return Continue (diag stub).
        let r1 = trigger.run(&ctx).await;
        let r2 = trigger.run(&ctx).await;
        assert_eq!(r1, TriggerOutcome::Continue);
        assert_eq!(r2, TriggerOutcome::Continue);
        assert_eq!(budget.current(milestone.id), 2);

        // Third invocation must Halt before touching the diagnostic.
        let r3 = trigger.run(&ctx).await;
        match r3 {
            TriggerOutcome::Halt(reason) => {
                assert!(
                    reason.contains("budget exceeded"),
                    "halt reason should mention budget; got: {reason}"
                );
            }
            other => panic!("expected Halt, got {other:?}"),
        }
        // Counter must not have been incremented on the halted fire.
        assert_eq!(budget.current(milestone.id), 2);
    }

    /// `OnTaskFailed` fires without a milestone in the context; the
    /// trigger resolves the milestone from the failing task.
    #[tokio::test]
    async fn resolves_milestone_from_failing_task() {
        let orch = make_orchestrator();
        let project = orch
            .create_project(Goal::new("resolve smoke", swell_core::TaskId::new()))
            .await;
        let milestone = orch.create_milestone(project.id, "m".into()).await.unwrap();
        let task = orch
            .create_task("widget".into(), vec!["src/widget.rs".into()])
            .await
            .unwrap();
        orch.assign_task_to_milestone(task.id, milestone.id)
            .await
            .unwrap();

        let target = MilestoneId::new();
        let diagnostic = Arc::new(StubDiagnosticResearcher::with_queue(vec![
            Handoff::Replan {
                milestone: target,
                reason: "redo".into(),
            },
        ]));
        let budget = Arc::new(ResearcherBudget::new(2));
        let trigger = ResearcherTrigger::with_default_stages(
            Arc::downgrade(&orch),
            diagnostic,
            budget.clone(),
        );

        let outcome = trigger.run(&task_failed_ctx(task.id)).await;
        assert_eq!(outcome, TriggerOutcome::Reroute(target));
        // Budget should be counted against the *task's* milestone, not
        // the rerouted target.
        assert_eq!(budget.current(milestone.id), 1);
        assert_eq!(budget.current(target), 0);
    }

    /// `OnTaskFailed` for a loose task (no milestone) is a no-op:
    /// trigger returns `Continue` without invoking the diagnostic.
    #[tokio::test]
    async fn loose_task_skips_diagnostic() {
        let orch = make_orchestrator();
        let task = orch
            .create_task("loose".into(), vec!["src/x.rs".into()])
            .await
            .unwrap();

        // Diagnostic returns Abandon — if the trigger invoked it we
        // would see Halt, but we expect Continue because there's no
        // milestone to count or reroute to.
        let diagnostic = Arc::new(StubDiagnosticResearcher::with_queue(vec![
            Handoff::Abandon {
                milestone: MilestoneId::new(),
                reason: "should not surface".into(),
            },
        ]));
        let budget = Arc::new(ResearcherBudget::new(2));
        let trigger =
            ResearcherTrigger::with_default_stages(Arc::downgrade(&orch), diagnostic, budget);

        let outcome = trigger.run(&task_failed_ctx(task.id)).await;
        assert_eq!(outcome, TriggerOutcome::Continue);
    }

    /// Abandon path: stub returns Abandon, trigger surfaces Halt with
    /// the reason intact.
    #[tokio::test]
    async fn abandon_surfaces_halt_reason() {
        let orch = make_orchestrator();
        let project = orch
            .create_project(Goal::new("abandon smoke", swell_core::TaskId::new()))
            .await;
        let milestone = orch.create_milestone(project.id, "m".into()).await.unwrap();
        let diagnostic = Arc::new(StubDiagnosticResearcher::with_queue(vec![
            Handoff::Abandon {
                milestone: milestone.id,
                reason: "no path forward".into(),
            },
        ]));
        let budget = Arc::new(ResearcherBudget::new(2));
        let trigger =
            ResearcherTrigger::with_default_stages(Arc::downgrade(&orch), diagnostic, budget);
        let outcome = trigger
            .run(&milestone_blocked_ctx(project.id, milestone.id))
            .await;
        match outcome {
            TriggerOutcome::Halt(r) => assert_eq!(r, "no path forward"),
            other => panic!("expected Halt, got {other:?}"),
        }
    }

    /// Factory honors `max_invocations` override and produces a working
    /// trigger.
    #[tokio::test]
    async fn factory_honors_max_invocations_override() {
        let orch = make_orchestrator();
        let project = orch
            .create_project(Goal::new("factory smoke", swell_core::TaskId::new()))
            .await;
        let milestone = orch.create_milestone(project.id, "m".into()).await.unwrap();

        let mut factories = TriggerFactoryRegistry::new();
        register_default_researcher_factory(&mut factories, Arc::downgrade(&orch));

        let cfg: crate::trigger_config::TriggerConfig = serde_json::from_str(
            r#"{ "researcher": { "stages": ["OnMilestoneBlocked"], "config": { "max_invocations": 1 } } }"#,
        )
        .unwrap();
        let loaded = crate::trigger_config::build_triggers(&cfg, &factories);
        assert_eq!(loaded.built.len(), 1, "researcher factory must build");
        let trigger = loaded.built.into_iter().next().unwrap();
        assert_eq!(trigger.name(), "researcher");

        // cap=1 → first fire is Continue, second fire is Halt.
        let ctx = milestone_blocked_ctx(project.id, milestone.id);
        let r1 = trigger.run(&ctx).await;
        let r2 = trigger.run(&ctx).await;
        assert_eq!(r1, TriggerOutcome::Continue);
        match r2 {
            TriggerOutcome::Halt(reason) => assert!(reason.contains("budget exceeded")),
            other => panic!("expected halt at cap=1, got {other:?}"),
        }
    }

    /// Factory with no config blob falls back to the default cap.
    #[test]
    fn factory_default_cap_when_config_absent() {
        let orch = make_orchestrator();
        let mut factories = TriggerFactoryRegistry::new();
        register_default_researcher_factory(&mut factories, Arc::downgrade(&orch));
        let names = factories.known_names();
        assert!(names.contains(&"researcher"));
        // Verifying the cap path is exercised by the override test; this
        // probe just proves the factory registered under the right name.
    }

    // ------------------------------------------------------------------
    // LlmDiagnosticResearcher tests
    // ------------------------------------------------------------------

    use std::sync::Mutex as StdMutex;
    use swell_core::traits::LlmStopReason;
    use swell_core::{LlmBackend as CoreLlmBackend, LlmResponse, LlmToolDefinition};
    use swell_core::{StreamEvent, SwellError};

    /// Minimal LLM mock that returns a fixed response and captures the
    /// messages it was invoked with. Lets tests pin both the parser
    /// and the prompt shape without pulling in the heavier
    /// `ScenarioMockLlm`.
    struct CapturingLlm {
        model: String,
        response: String,
        should_fail: bool,
        captured: Arc<StdMutex<Vec<Vec<LlmMessage>>>>,
    }

    impl CapturingLlm {
        fn new(response: impl Into<String>) -> Self {
            Self {
                model: "test-diagnostic".to_string(),
                response: response.into(),
                should_fail: false,
                captured: Arc::new(StdMutex::new(Vec::new())),
            }
        }
        fn failing() -> Self {
            Self {
                model: "test-diagnostic".to_string(),
                response: String::new(),
                should_fail: true,
                captured: Arc::new(StdMutex::new(Vec::new())),
            }
        }
        fn captured(&self) -> Arc<StdMutex<Vec<Vec<LlmMessage>>>> {
            Arc::clone(&self.captured)
        }
    }

    #[async_trait]
    impl CoreLlmBackend for CapturingLlm {
        fn model(&self) -> &str {
            &self.model
        }
        async fn chat(
            &self,
            messages: Vec<LlmMessage>,
            _tools: Option<Vec<LlmToolDefinition>>,
            _config: LlmConfig,
        ) -> Result<LlmResponse, SwellError> {
            self.captured.lock().unwrap().push(messages);
            if self.should_fail {
                return Err(SwellError::LlmError("scripted failure".into()));
            }
            Ok(LlmResponse {
                content: self.response.clone(),
                stop_reason: Some(LlmStopReason::EndTurn),
                ..Default::default()
            })
        }
        async fn health_check(&self) -> bool {
            true
        }
        async fn stream(
            &self,
            _messages: Vec<LlmMessage>,
            _tools: Option<Vec<LlmToolDefinition>>,
            _config: LlmConfig,
        ) -> Result<
            std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamEvent, SwellError>> + Send>>,
            SwellError,
        > {
            unimplemented!("CapturingLlm doesn't support streaming")
        }
    }

    fn diag_ctx(milestone: MilestoneId) -> DiagnosticContext {
        DiagnosticContext {
            stage: Stage::OnMilestoneBlocked,
            project: None,
            milestone,
            failing_task: None,
            reason: String::new(),
            prior_invocations: 0,
            escalation: false,
        }
    }

    /// `verdict=replan` with an explicit milestone routes to that target.
    #[tokio::test]
    async fn llm_replan_with_target_routes_to_target() {
        let source = MilestoneId::new();
        let target = MilestoneId::new();
        let response = format!(
            r#"{{"verdict":"replan","reason":"flaky lock","replan_milestone":"{target}"}}"#
        );
        let llm = Arc::new(CapturingLlm::new(response));
        let orch = make_orchestrator();
        let diag =
            LlmDiagnosticResearcher::new(Arc::downgrade(&orch), llm.clone() as Arc<dyn LlmBackend>);
        let handoff = diag.diagnose(&diag_ctx(source)).await;
        match handoff {
            Handoff::Replan { milestone, reason } => {
                assert_eq!(milestone, target);
                assert_eq!(reason, "flaky lock");
            }
            other => panic!("expected Replan, got {other:?}"),
        }
    }

    /// `verdict=replan` without a target falls back to the source
    /// milestone so the scheduler re-walks it.
    #[tokio::test]
    async fn llm_replan_without_target_defaults_to_source() {
        let source = MilestoneId::new();
        let response = r#"{"verdict":"replan","reason":"retry"}"#;
        let llm = Arc::new(CapturingLlm::new(response));
        let orch = make_orchestrator();
        let diag = LlmDiagnosticResearcher::new(Arc::downgrade(&orch), llm as Arc<dyn LlmBackend>);
        let handoff = diag.diagnose(&diag_ctx(source)).await;
        match handoff {
            Handoff::Replan { milestone, .. } => assert_eq!(milestone, source),
            other => panic!("expected Replan, got {other:?}"),
        }
    }

    /// `verdict=replan` with a malformed UUID falls back to the source.
    #[tokio::test]
    async fn llm_replan_with_garbage_target_defaults_to_source() {
        let source = MilestoneId::new();
        let response = r#"{"verdict":"replan","reason":"retry","replan_milestone":"not-a-uuid"}"#;
        let llm = Arc::new(CapturingLlm::new(response));
        let orch = make_orchestrator();
        let diag = LlmDiagnosticResearcher::new(Arc::downgrade(&orch), llm as Arc<dyn LlmBackend>);
        let handoff = diag.diagnose(&diag_ctx(source)).await;
        match handoff {
            Handoff::Replan { milestone, .. } => assert_eq!(milestone, source),
            other => panic!("expected Replan defaulting to source, got {other:?}"),
        }
    }

    /// `verdict=abandon` surfaces Abandon with the reason intact.
    #[tokio::test]
    async fn llm_abandon_translates_to_abandon() {
        let source = MilestoneId::new();
        let response =
            r#"{"verdict":"abandon","reason":"requirement infeasible without external API"}"#;
        let llm = Arc::new(CapturingLlm::new(response));
        let orch = make_orchestrator();
        let diag = LlmDiagnosticResearcher::new(Arc::downgrade(&orch), llm as Arc<dyn LlmBackend>);
        let handoff = diag.diagnose(&diag_ctx(source)).await;
        match handoff {
            Handoff::Abandon { milestone, reason } => {
                assert_eq!(milestone, source);
                assert!(reason.contains("requirement infeasible"));
            }
            other => panic!("expected Abandon, got {other:?}"),
        }
    }

    /// `verdict=continue` round-trips to `Handoff::Continue`.
    #[tokio::test]
    async fn llm_continue_translates_to_continue() {
        let response = r#"{"verdict":"continue","reason":"already retrying"}"#;
        let llm = Arc::new(CapturingLlm::new(response));
        let orch = make_orchestrator();
        let diag = LlmDiagnosticResearcher::new(Arc::downgrade(&orch), llm as Arc<dyn LlmBackend>);
        let handoff = diag.diagnose(&diag_ctx(MilestoneId::new())).await;
        assert_eq!(handoff, Handoff::Continue);
    }

    /// `verdict=split_milestone` round-trips through the wire to the
    /// `split_milestone` verdict with `sub_plans` is parsed into a
    /// `Handoff::SplitMilestone` carrying the source milestone and the
    /// decoded plans. Pin the wire-format contract that
    /// `Orchestrator::split_milestone` consumes.
    #[tokio::test]
    async fn llm_split_milestone_yields_split_handoff_with_sub_plans() {
        let source = MilestoneId::new();
        let response = r#"{"verdict":"split_milestone","reason":"scope too wide","sub_plans":[{"name":"part-a","description":"focus on lexer"},{"name":"part-b","parallel_tasks":true}]}"#;
        let llm = Arc::new(CapturingLlm::new(response));
        let orch = make_orchestrator();
        let diag = LlmDiagnosticResearcher::new(Arc::downgrade(&orch), llm as Arc<dyn LlmBackend>);
        let handoff = diag.diagnose(&diag_ctx(source)).await;
        match handoff {
            Handoff::SplitMilestone { milestone, into } => {
                assert_eq!(milestone, source);
                assert_eq!(into.len(), 2, "both sub_plans should round-trip");
                assert_eq!(into[0].name, "part-a");
                assert_eq!(into[0].description.as_deref(), Some("focus on lexer"));
                assert!(!into[0].parallel_tasks, "default parallel_tasks is off");
                assert_eq!(into[1].name, "part-b");
                assert!(
                    into[1].parallel_tasks,
                    "explicit `parallel_tasks: true` round-trips"
                );
            }
            other => panic!("expected SplitMilestone, got {other:?}"),
        }
    }

    /// `split_milestone` verdict with no `sub_plans` array is a malformed
    /// recovery (you can't split into zero milestones) and degrades to
    /// `Continue`. Pin this so a future "infer a single child" shortcut
    /// has to be added deliberately.
    #[tokio::test]
    async fn llm_split_milestone_without_sub_plans_falls_back_to_continue() {
        let source = MilestoneId::new();
        let response = r#"{"verdict":"split_milestone","reason":"scope too wide"}"#;
        let llm = Arc::new(CapturingLlm::new(response));
        let orch = make_orchestrator();
        let diag = LlmDiagnosticResearcher::new(Arc::downgrade(&orch), llm as Arc<dyn LlmBackend>);
        let handoff = diag.diagnose(&diag_ctx(source)).await;
        assert_eq!(
            handoff,
            Handoff::Continue,
            "empty sub_plans on split_milestone must degrade to Continue"
        );
    }

    /// Markdown fences around the JSON object are stripped. Real models
    /// sometimes ignore the no-fence instruction; the parser must be
    /// tolerant or every fenced response would degrade to Continue.
    #[tokio::test]
    async fn llm_response_with_markdown_fences_is_parsed() {
        let response = "```json\n{\"verdict\":\"continue\",\"reason\":\"ok\"}\n```";
        let llm = Arc::new(CapturingLlm::new(response));
        let orch = make_orchestrator();
        let diag = LlmDiagnosticResearcher::new(Arc::downgrade(&orch), llm as Arc<dyn LlmBackend>);
        let handoff = diag.diagnose(&diag_ctx(MilestoneId::new())).await;
        assert_eq!(handoff, Handoff::Continue);
    }

    /// Malformed JSON (model emits prose) degrades safely to Continue.
    #[tokio::test]
    async fn llm_malformed_response_falls_back_to_continue() {
        let response = "I think you should retry but I am not sure.";
        let llm = Arc::new(CapturingLlm::new(response));
        let orch = make_orchestrator();
        let diag = LlmDiagnosticResearcher::new(Arc::downgrade(&orch), llm as Arc<dyn LlmBackend>);
        let handoff = diag.diagnose(&diag_ctx(MilestoneId::new())).await;
        assert_eq!(handoff, Handoff::Continue);
    }

    /// Unknown verdict string degrades safely to Continue.
    #[tokio::test]
    async fn llm_unknown_verdict_falls_back_to_continue() {
        let response = r#"{"verdict":"escalate-to-human","reason":"unsure"}"#;
        let llm = Arc::new(CapturingLlm::new(response));
        let orch = make_orchestrator();
        let diag = LlmDiagnosticResearcher::new(Arc::downgrade(&orch), llm as Arc<dyn LlmBackend>);
        let handoff = diag.diagnose(&diag_ctx(MilestoneId::new())).await;
        assert_eq!(handoff, Handoff::Continue);
    }

    /// LLM error (network failure, quota exhaustion, etc.) degrades
    /// safely to Continue — a broken diagnostic must never block the
    /// orchestrator's failure path.
    #[tokio::test]
    async fn llm_error_falls_back_to_continue() {
        let llm = Arc::new(CapturingLlm::failing());
        let orch = make_orchestrator();
        let diag = LlmDiagnosticResearcher::new(Arc::downgrade(&orch), llm as Arc<dyn LlmBackend>);
        let handoff = diag.diagnose(&diag_ctx(MilestoneId::new())).await;
        assert_eq!(handoff, Handoff::Continue);
    }

    /// The user prompt mentions the failing task's id, description, and
    /// the first validator error. Pin the shape so a future prompt
    /// refactor doesn't silently drop the most useful evidence.
    #[tokio::test]
    async fn llm_prompt_carries_failing_task_evidence() {
        let orch = make_orchestrator();
        let project = orch
            .create_project(Goal::new("prompt smoke", swell_core::TaskId::new()))
            .await;
        let milestone = orch.create_milestone(project.id, "m".into()).await.unwrap();
        let task = orch
            .create_task("implement widget".into(), vec!["src/widget.rs".into()])
            .await
            .unwrap();
        orch.assign_task_to_milestone(task.id, milestone.id)
            .await
            .unwrap();
        {
            let sm = orch.state_machine();
            let sm = sm.read().await;
            sm.with_task_mut(task.id, |t| {
                t.validation_result = Some(swell_core::ValidationResult {
                    passed: false,
                    lint_passed: true,
                    tests_passed: false,
                    security_passed: true,
                    ai_review_passed: true,
                    errors: vec!["test widget::tests::it_works failed".into()],
                    warnings: vec![],
                });
                Ok(())
            })
            .unwrap();
        }

        let task_with_validation = orch.get_task(task.id).await.unwrap();
        let ctx = DiagnosticContext {
            stage: Stage::OnTaskFailed,
            project: Some(project.id),
            milestone: milestone.id,
            failing_task: Some(task_with_validation),
            reason: "validator rejected".into(),
            prior_invocations: 0,
            escalation: false,
        };

        let llm = Arc::new(CapturingLlm::new(r#"{"verdict":"continue","reason":"ok"}"#));
        let captured = llm.captured();
        let diag =
            LlmDiagnosticResearcher::new(Arc::downgrade(&orch), llm.clone() as Arc<dyn LlmBackend>);
        let _ = diag.diagnose(&ctx).await;

        let calls = captured.lock().unwrap();
        assert_eq!(
            calls.len(),
            1,
            "diagnostic must invoke the LLM exactly once"
        );
        let messages = &calls[0];
        assert_eq!(messages.len(), 2, "system + user turns");
        assert!(matches!(messages[0].role, LlmRole::System));
        assert!(messages[0].content.contains("\"verdict\""));
        assert!(matches!(messages[1].role, LlmRole::User));
        let user = &messages[1].content;
        assert!(user.contains("task failed"), "stage label missing: {user}");
        assert!(
            user.contains(&task.id.to_string()),
            "failing task id missing: {user}"
        );
        assert!(
            user.contains("implement widget"),
            "task description missing: {user}"
        );
        assert!(
            user.contains("it_works"),
            "validator error missing from prompt: {user}"
        );
        assert!(
            user.contains("validator rejected"),
            "reason field missing: {user}"
        );
    }

    /// `build_user_prompt` mentions the prior_invocations counter so
    /// the diagnostic can change its tone on the second pass before
    /// the budget halts it.
    #[test]
    fn user_prompt_includes_prior_invocations() {
        let ctx = DiagnosticContext {
            stage: Stage::OnMilestoneBlocked,
            project: None,
            milestone: MilestoneId::new(),
            failing_task: None,
            reason: String::new(),
            prior_invocations: 1,
            escalation: false,
        };
        let prompt = LlmDiagnosticResearcher::build_user_prompt(&ctx);
        assert!(prompt.contains("Prior researcher invocations"));
        assert!(prompt.contains("1"));
    }

    /// `build_user_prompt` adds an escalation preamble when the fire
    /// was driven by `LoopIntervention::Escalation`, so the LLM can
    /// bias toward `replan` / `split_milestone`. Pin the wording so
    /// a future refactor of the prompt doesn't drop the signal.
    #[test]
    fn user_prompt_carries_escalation_preamble_when_set() {
        let ctx = DiagnosticContext {
            stage: Stage::OnTaskFailed,
            project: None,
            milestone: MilestoneId::new(),
            failing_task: None,
            reason: "loop detector escalation".into(),
            prior_invocations: 0,
            escalation: true,
        };
        let prompt = LlmDiagnosticResearcher::build_user_prompt(&ctx);
        assert!(
            prompt.contains("Escalation source: loop detector"),
            "escalation preamble missing from prompt: {prompt}"
        );
        // Negative control: same builder without escalation must omit
        // the preamble.
        let plain_ctx = DiagnosticContext {
            escalation: false,
            reason: String::new(),
            ..ctx
        };
        let plain = LlmDiagnosticResearcher::build_user_prompt(&plain_ctx);
        assert!(
            !plain.contains("Escalation source"),
            "escalation preamble must NOT appear when ctx.escalation is false"
        );
    }

    /// `ResearcherTrigger::run` propagates `ctx.escalation` into the
    /// `DiagnosticContext`. We probe this through a capturing stub
    /// diagnostic — without this wiring, the researcher could never
    /// see that the fire was loop-driven.
    #[tokio::test]
    async fn run_propagates_escalation_flag_into_diagnostic_context() {
        use crate::triggers::{Stage, TriggerContext};
        use std::sync::Mutex as StdMutex;

        struct CapturingDiagnostic {
            seen_escalation: Arc<StdMutex<Option<bool>>>,
        }
        #[async_trait]
        impl DiagnosticResearcher for CapturingDiagnostic {
            async fn diagnose(&self, ctx: &DiagnosticContext) -> Handoff {
                *self.seen_escalation.lock().unwrap() = Some(ctx.escalation);
                Handoff::Continue
            }
        }

        let orch = make_orchestrator();
        let project = orch
            .create_project(Goal::new("escalation flag", swell_core::TaskId::new()))
            .await;
        let milestone = orch.create_milestone(project.id, "m".into()).await.unwrap();
        let task = orch
            .create_task("escalated task".into(), vec![])
            .await
            .unwrap();
        orch.assign_task_to_milestone(task.id, milestone.id)
            .await
            .unwrap();

        let seen = Arc::new(StdMutex::new(None));
        let diagnostic = Arc::new(CapturingDiagnostic {
            seen_escalation: Arc::clone(&seen),
        });
        let budget = Arc::new(ResearcherBudget::new(4));
        let trigger =
            ResearcherTrigger::with_default_stages(Arc::downgrade(&orch), diagnostic, budget);

        // Fire with escalation = true.
        let ctx_escalated =
            TriggerContext::for_task(Stage::OnTaskFailed, task.id).with_escalation(true);
        let _ = trigger.run(&ctx_escalated).await;
        assert_eq!(
            *seen.lock().unwrap(),
            Some(true),
            "diagnostic must see ctx.escalation = true when run() was called with escalation"
        );

        // And again with escalation = false to confirm the default path
        // doesn't smuggle in true.
        let ctx_plain = TriggerContext::for_task(Stage::OnTaskFailed, task.id);
        let _ = trigger.run(&ctx_plain).await;
        assert_eq!(
            *seen.lock().unwrap(),
            Some(false),
            "diagnostic must see ctx.escalation = false on a plain fire"
        );
    }

    /// `parse_response` is a public pure helper — pin a couple of edge
    /// cases that the higher-level integration tests cover too, so a
    /// future refactor of the diagnose flow doesn't silently break the
    /// parser contract.
    #[test]
    fn parse_response_handles_extra_whitespace_and_fences() {
        let source = MilestoneId::new();
        let h = LlmDiagnosticResearcher::parse_response(
            "  \n  ```json\n{\"verdict\":\"abandon\",\"reason\":\"no\"}\n```  \n",
            source,
        );
        match h {
            Handoff::Abandon { milestone, reason } => {
                assert_eq!(milestone, source);
                assert_eq!(reason, "no");
            }
            other => panic!("expected Abandon, got {other:?}"),
        }
    }

    /// LLM-backed factory builds a working trigger that returns
    /// `Reroute` end-to-end when the model replies "replan".
    #[tokio::test]
    async fn llm_factory_end_to_end() {
        let orch = make_orchestrator();
        let project = orch
            .create_project(Goal::new("llm factory smoke", swell_core::TaskId::new()))
            .await;
        let source = orch
            .create_milestone(project.id, "source".into())
            .await
            .unwrap();
        let target = orch
            .create_milestone(project.id, "target".into())
            .await
            .unwrap();

        let llm = Arc::new(CapturingLlm::new(format!(
            r#"{{"verdict":"replan","reason":"retry","replan_milestone":"{}"}}"#,
            target.id
        ))) as Arc<dyn LlmBackend>;
        let mut factories = TriggerFactoryRegistry::new();
        register_llm_researcher_factory(&mut factories, Arc::downgrade(&orch), llm);

        let cfg: crate::trigger_config::TriggerConfig =
            serde_json::from_str(r#"{ "researcher": { "stages": ["OnMilestoneBlocked"] } }"#)
                .unwrap();
        let loaded = crate::trigger_config::build_triggers(&cfg, &factories);
        assert_eq!(loaded.built.len(), 1);
        let trigger = loaded.built.into_iter().next().unwrap();

        let ctx = milestone_blocked_ctx(project.id, source.id);
        let outcome = trigger.run(&ctx).await;
        assert_eq!(outcome, TriggerOutcome::Reroute(target.id));
    }

    // ------------------------------------------------------------------
    // Tool-loop diagnostic tests
    // ------------------------------------------------------------------

    use swell_core::traits::{Tool as CoreTool, ToolBehavioralHints};
    use swell_core::{PermissionTier, ToolRiskLevel};
    use swell_tools::registry::{ToolCategory, ToolLayer};

    /// Scripted LLM for tool-loop tests. Each chat() call returns the
    /// next step from the queue. Captures the messages it was given so
    /// tests can assert the conversation shape.
    struct ScriptedLlm {
        model: String,
        steps: StdMutex<Vec<ScriptedStep>>,
        captured: Arc<StdMutex<Vec<Vec<LlmMessage>>>>,
    }

    #[derive(Debug, Clone)]
    enum ScriptedStep {
        ToolUse {
            id: &'static str,
            name: &'static str,
            arguments: serde_json::Value,
        },
        Text(String),
        Fail(String),
    }

    impl ScriptedLlm {
        fn new(steps: Vec<ScriptedStep>) -> Self {
            Self {
                model: "scripted".to_string(),
                steps: StdMutex::new(steps),
                captured: Arc::new(StdMutex::new(Vec::new())),
            }
        }
        fn captured(&self) -> Arc<StdMutex<Vec<Vec<LlmMessage>>>> {
            Arc::clone(&self.captured)
        }
    }

    #[async_trait]
    impl CoreLlmBackend for ScriptedLlm {
        fn model(&self) -> &str {
            &self.model
        }
        async fn chat(
            &self,
            messages: Vec<LlmMessage>,
            _tools: Option<Vec<LlmToolDefinition>>,
            _config: LlmConfig,
        ) -> Result<LlmResponse, SwellError> {
            self.captured.lock().unwrap().push(messages);
            let mut steps = self.steps.lock().unwrap();
            if steps.is_empty() {
                return Err(SwellError::LlmError("scripted scenario exhausted".into()));
            }
            match steps.remove(0) {
                ScriptedStep::Text(content) => Ok(LlmResponse {
                    content,
                    stop_reason: Some(LlmStopReason::EndTurn),
                    ..Default::default()
                }),
                ScriptedStep::ToolUse {
                    id,
                    name,
                    arguments,
                } => Ok(LlmResponse {
                    content: String::new(),
                    tool_calls: Some(vec![LlmToolCall {
                        id: id.to_string(),
                        name: name.to_string(),
                        arguments,
                    }]),
                    stop_reason: Some(LlmStopReason::ToolUse),
                    ..Default::default()
                }),
                ScriptedStep::Fail(msg) => Err(SwellError::LlmError(msg)),
            }
        }
        async fn health_check(&self) -> bool {
            true
        }
        async fn stream(
            &self,
            _messages: Vec<LlmMessage>,
            _tools: Option<Vec<LlmToolDefinition>>,
            _config: LlmConfig,
        ) -> Result<
            std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamEvent, SwellError>> + Send>>,
            SwellError,
        > {
            unimplemented!("ScriptedLlm doesn't support streaming")
        }
    }

    /// Read-only fake tool that always returns the same text payload.
    struct FakeReadFile {
        response: String,
    }

    #[async_trait]
    impl CoreTool for FakeReadFile {
        fn name(&self) -> &str {
            "fake_read_file"
        }
        fn description(&self) -> String {
            "read a file (test fake, returns fixed payload)".to_string()
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"]
            })
        }
        fn risk_level(&self) -> ToolRiskLevel {
            ToolRiskLevel::Read
        }
        fn permission_tier(&self) -> PermissionTier {
            PermissionTier::Auto
        }
        fn behavioral_hints(&self) -> ToolBehavioralHints {
            ToolBehavioralHints {
                read_only_hint: true,
                destructive_hint: false,
                idempotent_hint: true,
            }
        }
        async fn execute(&self, _arguments: serde_json::Value) -> Result<ToolOutput, SwellError> {
            Ok(ToolOutput {
                is_error: false,
                content: vec![ToolResultContent::Text(self.response.clone())],
            })
        }
    }

    /// Destructive fake tool — must be filtered out by the read-only
    /// gate even if the registry registered it.
    struct FakeShellExec;

    #[async_trait]
    impl CoreTool for FakeShellExec {
        fn name(&self) -> &str {
            "fake_shell_exec"
        }
        fn description(&self) -> String {
            "execute a shell command (test fake)".to_string()
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        fn risk_level(&self) -> ToolRiskLevel {
            ToolRiskLevel::Destructive
        }
        fn permission_tier(&self) -> PermissionTier {
            PermissionTier::Deny
        }
        fn behavioral_hints(&self) -> ToolBehavioralHints {
            ToolBehavioralHints {
                read_only_hint: false,
                destructive_hint: true,
                idempotent_hint: false,
            }
        }
        async fn execute(&self, _arguments: serde_json::Value) -> Result<ToolOutput, SwellError> {
            // Should never be called by the diagnostic; if it is, the
            // test asserts the unreachable.
            unreachable!("destructive tool must never be executed by diagnostic")
        }
    }

    /// Tool that always errors on execute. Used to verify the
    /// diagnostic surfaces the error as `tool_result_is_error: true`
    /// without crashing.
    struct FakeFailingTool;

    #[async_trait]
    impl CoreTool for FakeFailingTool {
        fn name(&self) -> &str {
            "fake_failing_read"
        }
        fn description(&self) -> String {
            "always fails (test fake)".to_string()
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        fn risk_level(&self) -> ToolRiskLevel {
            ToolRiskLevel::Read
        }
        fn permission_tier(&self) -> PermissionTier {
            PermissionTier::Auto
        }
        fn behavioral_hints(&self) -> ToolBehavioralHints {
            ToolBehavioralHints {
                read_only_hint: true,
                destructive_hint: false,
                idempotent_hint: true,
            }
        }
        async fn execute(&self, _arguments: serde_json::Value) -> Result<ToolOutput, SwellError> {
            Err(SwellError::ToolExecutionFailed("scripted failure".into()))
        }
    }

    async fn registry_with_read_only_and_destructive(read_response: &str) -> Arc<ToolRegistry> {
        let registry = Arc::new(ToolRegistry::new());
        registry
            .register(
                FakeReadFile {
                    response: read_response.into(),
                },
                ToolCategory::Misc,
                ToolLayer::Builtin,
            )
            .await;
        registry
            .register(FakeShellExec, ToolCategory::Misc, ToolLayer::Builtin)
            .await;
        registry
    }

    /// Sanity probe: with no tools wired, the diagnostic still works
    /// the way the single-shot path always did. Pins behavior parity
    /// so the tool-loop refactor doesn't regress the no-tools case.
    #[tokio::test]
    async fn no_tools_still_single_shot() {
        let llm = Arc::new(CapturingLlm::new(r#"{"verdict":"continue","reason":"ok"}"#));
        let captured = llm.captured();
        let orch = make_orchestrator();
        let diag =
            LlmDiagnosticResearcher::new(Arc::downgrade(&orch), llm.clone() as Arc<dyn LlmBackend>);
        let handoff = diag.diagnose(&diag_ctx(MilestoneId::new())).await;
        assert_eq!(handoff, Handoff::Continue);
        assert_eq!(
            captured.lock().unwrap().len(),
            1,
            "exactly one LLM call when no tools are configured"
        );
    }

    /// One tool round, then verdict. End-to-end: scripted model asks
    /// to read a file, we execute the fake tool, the next turn's
    /// captured messages must include the assistant tool_use echo +
    /// user tool_result observation.
    #[tokio::test]
    async fn tool_loop_executes_one_call_then_verdicts() {
        let llm = Arc::new(ScriptedLlm::new(vec![
            ScriptedStep::ToolUse {
                id: "call_1",
                name: "fake_read_file",
                arguments: serde_json::json!({"path": "src/lib.rs"}),
            },
            ScriptedStep::Text(r#"{"verdict":"abandon","reason":"unrecoverable"}"#.to_string()),
        ]));
        let captured = llm.captured();
        let registry = registry_with_read_only_and_destructive("fn main() { /* stub */ }").await;
        let orch = make_orchestrator();
        let diag =
            LlmDiagnosticResearcher::new(Arc::downgrade(&orch), llm.clone() as Arc<dyn LlmBackend>)
                .with_tools(registry, None);

        let source = MilestoneId::new();
        let handoff = diag.diagnose(&diag_ctx(source)).await;
        match handoff {
            Handoff::Abandon { milestone, reason } => {
                assert_eq!(milestone, source);
                assert!(reason.contains("unrecoverable"));
            }
            other => panic!("expected Abandon after tool round, got {other:?}"),
        }

        let calls = captured.lock().unwrap();
        assert_eq!(calls.len(), 2, "tool round + verdict round = 2 LLM calls");
        // Round 2 must carry the tool round-trip in the conversation.
        let round_two = &calls[1];
        assert!(round_two.len() >= 4, "round 2 conversation: {round_two:#?}");
        // Find the assistant echo and the user tool_result.
        let echoed = round_two
            .iter()
            .any(|m| matches!(m.role, LlmRole::Assistant) && m.tool_calls.is_some());
        let result = round_two.iter().any(|m| {
            matches!(m.role, LlmRole::User) && m.tool_call_id.as_deref() == Some("call_1")
        });
        assert!(echoed, "assistant tool_use turn must be echoed");
        assert!(result, "user tool_result turn must be appended");
    }

    /// The destructive fake tool registers fine but the read-only
    /// filter excludes it from the definitions shipped to the model.
    /// If the model tries to call it anyway, the trigger blocks
    /// execution and surfaces an error observation.
    #[tokio::test]
    async fn tool_loop_blocks_destructive_tool_at_exec_time() {
        let llm = Arc::new(ScriptedLlm::new(vec![
            ScriptedStep::ToolUse {
                id: "call_1",
                name: "fake_shell_exec",
                arguments: serde_json::json!({}),
            },
            ScriptedStep::Text(r#"{"verdict":"continue","reason":"blocked"}"#.to_string()),
        ]));
        let registry = registry_with_read_only_and_destructive("payload").await;
        let orch = make_orchestrator();
        let diag =
            LlmDiagnosticResearcher::new(Arc::downgrade(&orch), llm.clone() as Arc<dyn LlmBackend>)
                .with_tools(registry, None);

        let handoff = diag.diagnose(&diag_ctx(MilestoneId::new())).await;
        // No panic, no exec — just Continue verdict from round 2.
        assert_eq!(handoff, Handoff::Continue);
    }

    /// Unknown tool name from the model surfaces as an error
    /// observation, the loop keeps going.
    #[tokio::test]
    async fn tool_loop_handles_unknown_tool_gracefully() {
        let llm = Arc::new(ScriptedLlm::new(vec![
            ScriptedStep::ToolUse {
                id: "call_1",
                name: "fake_does_not_exist",
                arguments: serde_json::json!({}),
            },
            ScriptedStep::Text(r#"{"verdict":"continue","reason":"unknown tool"}"#.to_string()),
        ]));
        let captured = llm.captured();
        let registry = registry_with_read_only_and_destructive("x").await;
        let orch = make_orchestrator();
        let diag =
            LlmDiagnosticResearcher::new(Arc::downgrade(&orch), llm.clone() as Arc<dyn LlmBackend>)
                .with_tools(registry, None);

        let handoff = diag.diagnose(&diag_ctx(MilestoneId::new())).await;
        assert_eq!(handoff, Handoff::Continue);
        let calls = captured.lock().unwrap();
        // Round 2 must carry an error tool_result.
        let round_two = &calls[1];
        let error_observation = round_two.iter().any(|m| {
            matches!(m.role, LlmRole::User) && m.tool_call_id.is_some() && m.tool_result_is_error
        });
        assert!(
            error_observation,
            "unknown tool must surface as tool_result_is_error"
        );
    }

    /// Tool execution error (read-only tool but its `execute` fails)
    /// surfaces as an error observation; the loop completes via the
    /// next scripted verdict.
    #[tokio::test]
    async fn tool_loop_handles_tool_execution_error() {
        let registry = Arc::new(ToolRegistry::new());
        registry
            .register(FakeFailingTool, ToolCategory::Misc, ToolLayer::Builtin)
            .await;

        let llm = Arc::new(ScriptedLlm::new(vec![
            ScriptedStep::ToolUse {
                id: "call_1",
                name: "fake_failing_read",
                arguments: serde_json::json!({}),
            },
            ScriptedStep::Text(r#"{"verdict":"continue","reason":"tool failed"}"#.to_string()),
        ]));
        let captured = llm.captured();
        let orch = make_orchestrator();
        let diag =
            LlmDiagnosticResearcher::new(Arc::downgrade(&orch), llm.clone() as Arc<dyn LlmBackend>)
                .with_tools(registry, None);
        let handoff = diag.diagnose(&diag_ctx(MilestoneId::new())).await;
        assert_eq!(handoff, Handoff::Continue);
        let calls = captured.lock().unwrap();
        let round_two = &calls[1];
        let saw_error = round_two
            .iter()
            .any(|m| m.tool_call_id.is_some() && m.tool_result_is_error);
        assert!(saw_error, "tool execution error must propagate");
    }

    /// When the model keeps asking for tools past the cap, the loop
    /// forces a verdict on the cap turn (parsing whatever the model
    /// emitted on that final turn). Here it emits no text, so we
    /// fall back to Continue.
    #[tokio::test]
    async fn tool_loop_iteration_cap_forces_continue_when_no_verdict() {
        // cap=1 → first turn is the cap turn; if the model emits
        // tool_use only on that turn, parse_response sees empty text
        // and returns Continue.
        let llm = Arc::new(ScriptedLlm::new(vec![ScriptedStep::ToolUse {
            id: "call_1",
            name: "fake_read_file",
            arguments: serde_json::json!({"path": "src/lib.rs"}),
        }]));
        let registry = registry_with_read_only_and_destructive("payload").await;
        let orch = make_orchestrator();
        let diag =
            LlmDiagnosticResearcher::new(Arc::downgrade(&orch), llm.clone() as Arc<dyn LlmBackend>)
                .with_tools(registry, Some(0));

        let handoff = diag.diagnose(&diag_ctx(MilestoneId::new())).await;
        assert_eq!(handoff, Handoff::Continue);
    }

    /// Tool-aware mode swaps in the tool-aware system prompt. Cheap
    /// pin so a future system-prompt rewrite doesn't silently drop
    /// the investigation preamble.
    #[tokio::test]
    async fn tool_aware_system_prompt_is_used_when_tools_wired() {
        let llm = Arc::new(ScriptedLlm::new(vec![ScriptedStep::Text(
            r#"{"verdict":"continue","reason":"ok"}"#.to_string(),
        )]));
        let captured = llm.captured();
        let registry = registry_with_read_only_and_destructive("payload").await;
        let orch = make_orchestrator();
        let diag =
            LlmDiagnosticResearcher::new(Arc::downgrade(&orch), llm.clone() as Arc<dyn LlmBackend>)
                .with_tools(registry, None);
        let _ = diag.diagnose(&diag_ctx(MilestoneId::new())).await;
        let calls = captured.lock().unwrap();
        let first = &calls[0];
        assert!(matches!(first[0].role, LlmRole::System));
        assert!(
            first[0].content.contains("READ-ONLY access"),
            "tool-aware system prompt missing: {}",
            first[0].content
        );
    }

    /// LLM error inside the tool loop degrades safely to Continue.
    #[tokio::test]
    async fn tool_loop_llm_error_falls_back_to_continue() {
        let llm = Arc::new(ScriptedLlm::new(vec![ScriptedStep::Fail(
            "network down".into(),
        )]));
        let registry = registry_with_read_only_and_destructive("x").await;
        let orch = make_orchestrator();
        let diag =
            LlmDiagnosticResearcher::new(Arc::downgrade(&orch), llm.clone() as Arc<dyn LlmBackend>)
                .with_tools(registry, None);
        let handoff = diag.diagnose(&diag_ctx(MilestoneId::new())).await;
        assert_eq!(handoff, Handoff::Continue);
    }

    // ------------------------------------------------------------------
    // Mode-switched factory tests
    // ------------------------------------------------------------------

    fn dummy_llm() -> Arc<dyn LlmBackend> {
        Arc::new(CapturingLlm::new(
            r#"{"verdict":"continue","reason":"factory probe"}"#,
        )) as Arc<dyn LlmBackend>
    }

    /// Default mode (no `mode` key) builds a stub-backed trigger:
    /// firing it does not call the LLM (the captured-llm count stays 0).
    #[tokio::test]
    async fn mode_switch_factory_defaults_to_stub() {
        let orch = make_orchestrator();
        let project = orch
            .create_project(Goal::new("mode-default smoke", swell_core::TaskId::new()))
            .await;
        let milestone = orch.create_milestone(project.id, "m".into()).await.unwrap();

        let llm = Arc::new(CapturingLlm::new(
            r#"{"verdict":"abandon","reason":"should not be invoked"}"#,
        ));
        let captured = llm.captured();
        let mut factories = TriggerFactoryRegistry::new();
        register_mode_switched_researcher_factory(
            &mut factories,
            Arc::downgrade(&orch),
            llm.clone() as Arc<dyn LlmBackend>,
            None,
        );

        let cfg: crate::trigger_config::TriggerConfig =
            serde_json::from_str(r#"{ "researcher": { "stages": ["OnMilestoneBlocked"] } }"#)
                .unwrap();
        let loaded = crate::trigger_config::build_triggers(&cfg, &factories);
        assert_eq!(loaded.built.len(), 1);
        let trigger = loaded.built.into_iter().next().unwrap();

        let outcome = trigger
            .run(&milestone_blocked_ctx(project.id, milestone.id))
            .await;
        assert_eq!(outcome, TriggerOutcome::Continue);
        assert_eq!(
            captured.lock().unwrap().len(),
            0,
            "stub mode must not invoke the LLM"
        );
    }

    /// `mode = "live"` with no registry runs single-shot LLM.
    #[tokio::test]
    async fn mode_switch_factory_live_without_tools_is_single_shot() {
        let orch = make_orchestrator();
        let project = orch
            .create_project(Goal::new("mode-live-bare smoke", swell_core::TaskId::new()))
            .await;
        let source = orch
            .create_milestone(project.id, "source".into())
            .await
            .unwrap();
        let target = orch
            .create_milestone(project.id, "target".into())
            .await
            .unwrap();

        let llm = Arc::new(CapturingLlm::new(format!(
            r#"{{"verdict":"replan","reason":"retry","replan_milestone":"{}"}}"#,
            target.id
        )));
        let captured = llm.captured();
        let mut factories = TriggerFactoryRegistry::new();
        register_mode_switched_researcher_factory(
            &mut factories,
            Arc::downgrade(&orch),
            llm.clone() as Arc<dyn LlmBackend>,
            None,
        );

        let cfg: crate::trigger_config::TriggerConfig = serde_json::from_str(
            r#"{ "researcher": { "stages": ["OnMilestoneBlocked"], "config": { "mode": "live", "use_tools": false } } }"#,
        )
        .unwrap();
        let loaded = crate::trigger_config::build_triggers(&cfg, &factories);
        let trigger = loaded.built.into_iter().next().unwrap();
        let outcome = trigger
            .run(&milestone_blocked_ctx(project.id, source.id))
            .await;
        assert_eq!(outcome, TriggerOutcome::Reroute(target.id));
        assert_eq!(
            captured.lock().unwrap().len(),
            1,
            "live mode without tools is single-shot"
        );
        // System prompt is the no-tools variant.
        let messages = &captured.lock().unwrap()[0];
        assert!(matches!(messages[0].role, LlmRole::System));
        assert!(
            !messages[0].content.contains("READ-ONLY access"),
            "single-shot system prompt must not carry tool-aware preamble"
        );
    }

    /// `mode = "live"` with a registry plumbs `with_tools` so the
    /// tool-aware system prompt is shipped and the diagnostic accepts
    /// tool_use responses.
    #[tokio::test]
    async fn mode_switch_factory_live_with_tools_uses_tool_loop() {
        let orch = make_orchestrator();
        let project = orch
            .create_project(Goal::new(
                "mode-live-tools smoke",
                swell_core::TaskId::new(),
            ))
            .await;
        let milestone = orch.create_milestone(project.id, "m".into()).await.unwrap();

        let llm = Arc::new(ScriptedLlm::new(vec![ScriptedStep::Text(
            r#"{"verdict":"continue","reason":"ok"}"#.to_string(),
        )]));
        let captured = llm.captured();
        let registry = registry_with_read_only_and_destructive("contents").await;
        let mut factories = TriggerFactoryRegistry::new();
        register_mode_switched_researcher_factory(
            &mut factories,
            Arc::downgrade(&orch),
            llm.clone() as Arc<dyn LlmBackend>,
            Some(registry),
        );

        let cfg: crate::trigger_config::TriggerConfig = serde_json::from_str(
            r#"{ "researcher": { "stages": ["OnMilestoneBlocked"], "config": { "mode": "live" } } }"#,
        )
        .unwrap();
        let loaded = crate::trigger_config::build_triggers(&cfg, &factories);
        let trigger = loaded.built.into_iter().next().unwrap();
        let outcome = trigger
            .run(&milestone_blocked_ctx(project.id, milestone.id))
            .await;
        assert_eq!(outcome, TriggerOutcome::Continue);
        let calls = captured.lock().unwrap();
        let first = &calls[0];
        assert!(matches!(first[0].role, LlmRole::System));
        assert!(
            first[0].content.contains("READ-ONLY access"),
            "tool-aware preamble missing: {}",
            first[0].content
        );
    }

    /// `mode = "live"` with `use_tools: true` but no registry warns
    /// and falls back to single-shot (no crash).
    #[tokio::test]
    async fn mode_switch_factory_live_with_tools_but_no_registry_falls_back() {
        let orch = make_orchestrator();
        let project = orch
            .create_project(Goal::new(
                "mode-live-no-reg smoke",
                swell_core::TaskId::new(),
            ))
            .await;
        let milestone = orch.create_milestone(project.id, "m".into()).await.unwrap();

        let llm = Arc::new(CapturingLlm::new(r#"{"verdict":"continue","reason":"ok"}"#));
        let mut factories = TriggerFactoryRegistry::new();
        register_mode_switched_researcher_factory(
            &mut factories,
            Arc::downgrade(&orch),
            llm.clone() as Arc<dyn LlmBackend>,
            None,
        );

        let cfg: crate::trigger_config::TriggerConfig = serde_json::from_str(
            r#"{ "researcher": { "stages": ["OnMilestoneBlocked"], "config": { "mode": "live", "use_tools": true } } }"#,
        )
        .unwrap();
        let loaded = crate::trigger_config::build_triggers(&cfg, &factories);
        let trigger = loaded.built.into_iter().next().unwrap();
        let outcome = trigger
            .run(&milestone_blocked_ctx(project.id, milestone.id))
            .await;
        assert_eq!(outcome, TriggerOutcome::Continue);
    }

    /// Unknown `mode` value warns and falls back to stub.
    #[tokio::test]
    async fn mode_switch_factory_unknown_mode_falls_back_to_stub() {
        let orch = make_orchestrator();
        let project = orch
            .create_project(Goal::new("mode-unknown smoke", swell_core::TaskId::new()))
            .await;
        let milestone = orch.create_milestone(project.id, "m".into()).await.unwrap();

        let llm = Arc::new(CapturingLlm::new(
            r#"{"verdict":"abandon","reason":"unused"}"#,
        ));
        let captured = llm.captured();
        let mut factories = TriggerFactoryRegistry::new();
        register_mode_switched_researcher_factory(
            &mut factories,
            Arc::downgrade(&orch),
            llm.clone() as Arc<dyn LlmBackend>,
            None,
        );

        let cfg: crate::trigger_config::TriggerConfig = serde_json::from_str(
            r#"{ "researcher": { "stages": ["OnMilestoneBlocked"], "config": { "mode": "yolo" } } }"#,
        )
        .unwrap();
        let loaded = crate::trigger_config::build_triggers(&cfg, &factories);
        let trigger = loaded.built.into_iter().next().unwrap();
        let outcome = trigger
            .run(&milestone_blocked_ctx(project.id, milestone.id))
            .await;
        assert_eq!(outcome, TriggerOutcome::Continue);
        assert_eq!(
            captured.lock().unwrap().len(),
            0,
            "unknown mode must fall back to stub (no LLM call)"
        );
    }

    /// `max_invocations` config still works on the mode-switched
    /// factory (the new config struct extends the old one).
    #[tokio::test]
    async fn mode_switch_factory_honors_max_invocations_in_live_mode() {
        let orch = make_orchestrator();
        let project = orch
            .create_project(Goal::new("mode-cap smoke", swell_core::TaskId::new()))
            .await;
        let milestone = orch.create_milestone(project.id, "m".into()).await.unwrap();

        let llm = Arc::new(CapturingLlm::new(r#"{"verdict":"continue","reason":"ok"}"#));
        let mut factories = TriggerFactoryRegistry::new();
        register_mode_switched_researcher_factory(
            &mut factories,
            Arc::downgrade(&orch),
            llm.clone() as Arc<dyn LlmBackend>,
            None,
        );

        let cfg: crate::trigger_config::TriggerConfig = serde_json::from_str(
            r#"{ "researcher": { "stages": ["OnMilestoneBlocked"], "config": { "mode": "live", "use_tools": false, "max_invocations": 1 } } }"#,
        )
        .unwrap();
        let loaded = crate::trigger_config::build_triggers(&cfg, &factories);
        let trigger = loaded.built.into_iter().next().unwrap();
        let ctx = milestone_blocked_ctx(project.id, milestone.id);
        let r1 = trigger.run(&ctx).await;
        let r2 = trigger.run(&ctx).await;
        assert_eq!(r1, TriggerOutcome::Continue);
        match r2 {
            TriggerOutcome::Halt(r) => assert!(r.contains("budget exceeded")),
            other => panic!("expected Halt at cap=1, got {other:?}"),
        }
    }

    /// Cheap probe that the factory registers under the expected name
    /// regardless of mode (matches the older default-cap test).
    #[test]
    fn mode_switch_factory_registers_under_researcher_name() {
        let orch = make_orchestrator();
        let mut factories = TriggerFactoryRegistry::new();
        register_mode_switched_researcher_factory(
            &mut factories,
            Arc::downgrade(&orch),
            dummy_llm(),
            None,
        );
        assert!(factories.known_names().contains(&"researcher"));
    }
}
