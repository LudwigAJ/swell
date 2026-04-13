# Recovery Recipes Policy

## What This Document Covers

Explains how ClawCode encodes failure taxonomies and structured recovery recipes, with exactly **one automatic recovery attempt before escalation** enforced as a first-class policy constraint. Grounded in `references/claw-code/rust/crates/runtime/src/recovery_recipes.rs`.

## Failure Taxonomy

The system defines six named `FailureScenario` variants — each representing a distinct failure class that has a known automatic recovery path:

| `FailureScenario` | Triggered by `WorkerFailureKind` |
|---|---|
| `TrustPromptUnresolved` | `TrustGate` — trust prompt appeared but did not resolve |
| `PromptMisdelivery` | `PromptDelivery` — prompt landed in shell or wrong target |
| `StaleBranch` | *(branch freshness check)* — main has commits not on this branch |
| `CompileRedCrossCrate` | *(compile check)* — cross-crate refactor left red builds |
| `McpHandshakeFailure` | `Protocol` — MCP server handshake failed |
| `PartialPluginStartup` | *(plugin lifecycle)* — plugin started but is degraded |
| `ProviderFailure` | `Provider` — provider degraded or session completed with `finish=unknown` and zero tokens |

The `FailureScenario::from_worker_failure_kind()` bridge maps `WorkerFailureKind` (emitted from `worker_boot.rs`) to the corresponding recovery scenario, so boot events flow directly into recovery dispatch without an ad hoc translation layer.

## The One-Attempt Policy

Every recipe enforces a hard `max_attempts: 1` constraint. This is not a coincidence or a default — it is an explicit policy statement: known failure modes should auto-heal once, then escalate if the recovery does not succeed.

From the module docstring comment in `recovery_recipes.rs`:

> Encodes known automatic recoveries for the six failure scenarios listed in ROADMAP item 8, and **enforces one automatic recovery attempt before escalation**.

The enforcement is implemented in `attempt_recovery()`:

```rust
// Enforce one automatic recovery attempt before escalation.
if *attempt_count >= recipe.max_attempts {
    let result = RecoveryResult::EscalationRequired {
        reason: format!(
            "max recovery attempts ({}) exceeded for {}",
            recipe.max_attempts, scenario
        ),
    };
    // ... emits RecoveryEvent::Escalated
    return result;
}
```

After the first recovery attempt, the attempt counter is incremented. A second call to `attempt_recovery()` for the same scenario immediately escalates rather than re-executing steps.

## Recovery Recipe Structure

Each `RecoveryRecipe` carries:

- `scenario: FailureScenario` — which failure class this recipe handles
- `steps: Vec<RecoveryStep>` — ordered sequence of recovery actions
- `max_attempts: u32` — always `1` in the current implementation
- `escalation_policy: EscalationPolicy` — one of `AlertHuman`, `LogAndContinue`, or `Abort`

### Step Types

The `RecoveryStep` enum enumerates all possible recovery actions:

- `AcceptTrustPrompt` — auto-resolve the trust prompt (used for `TrustPromptUnresolved`)
- `RedirectPromptToAgent` — re-route the prompt to the correct target (used for `PromptMisdelivery`)
- `RebaseBranch` — rebase against main to pick up missing commits (used for `StaleBranch`)
- `CleanBuild` — `cargo clean` + rebuild to resolve stale artifact issues (used for `StaleBranch` and `CompileRedCrossCrate`)
- `RetryMcpHandshake { timeout: u64 }` — retry the MCP handshake with a timeout
- `RestartPlugin { name: String }` — restart the stalled plugin by name (used for `PartialPluginStartup`)
- `RestartWorker` — restart the worker process (used for `ProviderFailure`)
- `EscalateToHuman { reason: String }` — this step itself triggers escalation (not currently used as a named step; escalation is the policy outcome)

### Escalation Policies

| Policy | Used for |
|---|---|
| `AlertHuman` | `TrustPromptUnresolved`, `PromptMisdelivery`, `StaleBranch`, `CompileRedCrossCrate`, `ProviderFailure` |
| `Abort` | `McpHandshakeFailure` |
| `LogAndContinue` | `PartialPluginStartup` |

`McpHandshakeFailure` uses `Abort` because a failed MCP handshake is considered non-recoverable at the worker level. `PartialPluginStartup` uses `LogAndContinue` because partial plugin degradation does not block the overall session — the other plugins continue functioning.

## Recipe Examples

**TrustPromptUnresolved** (1 step, `AlertHuman` escalation):
```rust
RecoveryRecipe {
    scenario: FailureScenario::TrustPromptUnresolved,
    steps: vec![RecoveryStep::AcceptTrustPrompt],
    max_attempts: 1,
    escalation_policy: EscalationPolicy::AlertHuman,
}
```

**StaleBranch** (2 steps, `AlertHuman` escalation):
```rust
RecoveryRecipe {
    scenario: FailureScenario::StaleBranch,
    steps: vec![RecoveryStep::RebaseBranch, RecoveryStep::CleanBuild],
    max_attempts: 1,
    escalation_policy: EscalationPolicy::AlertHuman,
}
```
The two-step sequence means: first attempt to bring the branch up to date by rebasing; if that succeeds, run a clean build to verify the state. Both steps run in the single allowed attempt. If any step fails, the attempt is considered failed and escalation fires immediately.

**PartialPluginStartup** (2 steps, `LogAndContinue` escalation):
```rust
RecoveryRecipe {
    scenario: FailureScenario::PartialPluginStartup,
    steps: vec![
        RecoveryStep::RestartPlugin { name: "stalled".to_string() },
        RecoveryStep::RetryMcpHandshake { timeout: 3000 },
    ],
    max_attempts: 1,
    escalation_policy: EscalationPolicy::LogAndContinue,
}
```

## Recovery Context and Event Emission

`RecoveryContext` is the minimal state tracker that carries:

- `attempts: HashMap<FailureScenario, u32>` — per-scenario attempt counters
- `events: Vec<RecoveryEvent>` — structured event log populated during recovery
- `fail_at_step: Option<usize>` — test hook that forces a specific step to fail

Every recovery action emits one or more structured `RecoveryEvent` values:

```rust
pub enum RecoveryEvent {
    RecoveryAttempted {
        scenario: FailureScenario,
        recipe: RecoveryRecipe,
        result: RecoveryResult,
    },
    RecoverySucceeded,
    RecoveryFailed,
    Escalated,
}
```

`RecoveryAttempted` carries the full recipe and the `RecoveryResult`, making the event log a complete audit trail of what was attempted and what outcome occurred.

The `fail_at_step` field is test-only — it lets tests simulate a recovery step failing at a specific index without needing to mock timing or external state.

## RecoveryResult Shapes

After each `attempt_recovery()` call, one of three `RecoveryResult` shapes is returned:

- `Recovered { steps_taken: u32 }` — all steps in the recipe succeeded
- `PartialRecovery { recovered: Vec<RecoveryStep>, remaining: Vec<RecoveryStep> }` — some steps succeeded, others did not
- `EscalationRequired { reason: String }` — recovery exhausted or failed at the first step

The `PartialRecovery` case is specifically designed for multi-step recipes like `PartialPluginStartup` where the first step may succeed but the second fails. The `remaining` vector gives the caller visibility into exactly which steps were not executed, enabling precise human handoff rather than a generic escalation message.

If all steps fail at index 0 (first step failure), escalation fires immediately rather than returning `PartialRecovery`. The test `first_step_failure_escalates_immediately` in `recovery_recipes.rs` verifies this boundary condition.

## Builder Lessons

### Structured event emission over string logging

The `RecoveryEvent` enum is structured and serializable to JSON. The test `emitted_events_include_structured_attempt_data` verifies that `serde_json::to_string(&ctx.events()[0])` round-trips cleanly. This means clawhip and downstream consumers can parse recovery events programmatically rather than regex-matching log lines.

### One attempt is a policy constraint, not a suggestion

`max_attempts: 1` is enforced in `attempt_recovery()` before any step is executed. There is no back-door path that retries silently — any second attempt escalates regardless of whether the first recovery "almost worked." This is the key architectural decision that makes the system recoverable without becoming a manual babysitting loop.

### Failure classification precedes recovery selection

The bridge `FailureScenario::from_worker_failure_kind()` ensures that recovery selection is driven by typed classification, not string matching on error messages. Adding a new `WorkerFailureKind` requires adding the corresponding `FailureScenario` variant and its recipe, making the taxonomy the source of truth for both failure detection and recovery dispatch.

### Multi-step recipes model sequential dependency

When `RebaseBranch` is the first step in a `StaleBranch` recipe, the intent is: rebase first, then verify the build is clean. A failure at step 1 (rebase) stops the sequence and escalates rather than attempting a clean build in an already-diverged state. This prevents recovery steps from running in an invalid context.

### Escalation policy is per-scenario, not global

`PartialPluginStartup` uses `LogAndContinue` because a degraded plugin does not mean the session is broken. `McpHandshakeFailure` uses `Abort` because a broken MCP handshake means the MCP surface is unavailable and continued operation would be undefined. The escalation policy encodes domain knowledge about what each failure category means for overall system health.

## Key Files

- `references/claw-code/rust/crates/runtime/src/recovery_recipes.rs` — module implementation
- `references/claw-code/rust/crates/runtime/src/worker_boot.rs` — `WorkerFailureKind` enum and `WorkerRegistry::observe_completion()` which classifies provider failures
- `references/claw-code/ROADMAP.md` — Phase 3 roadmap item 8 ("Recovery recipes for common failures") which specifies the one-attempt-before-escalation policy and the six scenarios

## Tests That Verify the Policy

- `each_scenario_has_a_matching_recipe` — every `FailureScenario` variant returns a recipe with at least one step and `max_attempts >= 1`
- `successful_recovery_returns_recovered_and_emits_events` — one successful attempt produces `RecoveryResult::Recovered` and two events (`RecoveryAttempted` + `RecoverySucceeded`)
- `escalation_after_max_attempts_exceeded` — first attempt succeeds, second attempt returns `EscalationRequired` with a reason mentioning `max recovery attempts`
- `partial_recovery_when_step_fails_midway` — multi-step recipe fails at step 1, produces `PartialRecovery` with one recovered step and one remaining step
- `first_step_failure_escalates_immediately` — failure at step index 0 escalates rather than returning `PartialRecovery`
- `worker_failure_kind_maps_to_failure_scenario` — the four `WorkerFailureKind` → `FailureScenario` mappings are verified
- `provider_failure_recovery_attempt_succeeds_then_escalates` — two attempts for `ProviderFailure`: first `Recovered`, second `EscalationRequired`
