# Builder Lessons from Worker States, Trust Resolution, Lane Events, and Recovery Recipes

## What This Document Is

This document extracts concrete design lessons from four interlocked mechanisms in ClawCode's Rust runtime:

- **`worker_boot.rs`** — explicit `WorkerStatus` state machine governing worker lifecycle
- **`trust_resolver.rs`** — path-based trust resolution with structured `TrustEvent` emission
- **`lane_events.rs`** — typed lane coordination event schema with `LaneEventName`, `LaneEventStatus`, and `LaneFailureClass`
- **`recovery_recipes.rs`** — structured `RecoveryRecipe` encoding with one-attempt-before-escalation policy

These mechanisms are not independent modules. They form a control-and-recovery pipeline: the worker state machine emits events that the lane event schema serializes, the trust resolver surfaces failures that the recovery recipes consume, and the recovery context tracks attempt counts that gate escalation. Understanding each mechanism in isolation is useful; understanding how they compose is the actual builder lesson.

The source documents for each mechanism are:

- `references/claw-code/rust/crates/runtime/src/worker_boot.rs`
- `references/claw-code/rust/crates/runtime/src/trust_resolver.rs`
- `references/claw-code/rust/crates/runtime/src/lane_events.rs`
- `references/claw-code/rust/crates/runtime/src/recovery_recipes.rs`

These same mechanisms appear in `coordination-system-philosophy.md` as the implementation evidence for the philosophy that humans set direction while claws perform labor. This document shows *how* that philosophy is encoded in code — not just that it is.

## Lesson 1: Explicit State Machines Beat Implicit Flags

The `WorkerStatus` enum in `worker_boot.rs` is a canonical example of "state machine first" (ROADMAP.md, principle 1). Six variants cover the full worker lifecycle:

```rust
pub enum WorkerStatus {
    Spawning,
    TrustRequired,
    ReadyForPrompt,
    Running,
    Finished,
    Failed,
}
```

The key design decision is that every transition is enforced through the registry API. `send_prompt()` requires `ReadyForPrompt` and returns an error for any other state. There is no way to silently send a prompt to a worker that has not cleared the trust gate. The state machine is not just documentation — it is the access control layer.

Compare this to a flag-based design: `is_trust_resolved: bool`, `is_prompt_sent: bool`, `is_running: bool`. A worker could be `trust_resolved=true`, `prompt_sent=true`, and `running=false` simultaneously — an inconsistent state the flag-based system has to reason about, but the enum disallows entirely.

**Builder takeaway:** When a process has a finite set of valid states and specific legal transitions between them, encode those states as a closed enum and gate every transition through a single registry or controller. This makes illegal states unrepresentable and forces every new code path to confront the state machine explicitly.

**Evidence:** `WorkerRegistry::send_prompt` at `worker_boot.rs` line 272 requires `WorkerStatus::ReadyForPrompt` as a precondition and returns `Err` if the worker is in any other state. The test `trust_prompt_blocks_non_allowlisted_worker_until_resolved` at `worker_boot.rs` line 370 verifies that `send_prompt` returns an error when called on a `TrustRequired` worker.

## Lesson 2: Typed Events Over Scraped Prose

`lane_events.rs` defines a `LaneEvent` schema with three orthogonal dimensions: `LaneEventName` (what happened), `LaneEventStatus` (current health), and `LaneFailureClass` (root cause). A `lane.blocked` event carries `status = Blocked` and a typed `LaneFailureClass` — not a freeform string that downstream consumers have to parse.

```rust
pub struct LaneEvent {
    pub event: LaneEventName,
    pub status: LaneEventStatus,
    pub emitted_at: String,
    pub failure_class: Option<LaneFailureClass>,
    pub detail: Option<String>,
    pub data: Option<Value>,
}
```

This directly implements ROADMAP.md principle 2: "Events over scraped prose." The wire format uses `#[serde(rename)]` annotations so the JSON serialization is stable and versioned independently of the Rust struct:

```json
{
  "event": "lane.blocked",
  "status": "blocked",
  "emittedAt": "2026-04-04T00:00:00Z",
  "failureClass": "mcp_startup"
}
```

The separation of `event` from `status` means dashboards can filter all currently-blocked lanes without caring which specific event caused the block, and retry policies can branch on `failure_class` without caring whether the lane ended with `Failed` or `Blocked`.

**Builder takeaway:** Typed events with orthogonal dimensions are more maintainable than freeform strings with embedded meaning. When designing a coordination system, define the event schema before writing the event emission code. A builder constructing a similar system should ask: what are the independent dimensions of each event? Can I filter on one dimension without decoding the others?

**Evidence:** `LaneEventName` at `lane_events.rs` lines 8–30 defines all event variants with `#[serde(rename)]` annotations. The tests `canonical_lane_event_names_serialize_to_expected_wire_values` and `failure_classes_cover_canonical_taxonomy_wire_values` at `lane_events.rs` lines 220–253 verify the wire format directly.

## Lesson 3: Recovery Recipes Are Data, Not Ad Hoc Code

`recovery_recipes.rs` encodes each `FailureScenario` as a `RecoveryRecipe` struct with `steps`, `max_attempts`, and `escalation_policy` fields — not as a match arm with embedded if-else logic. The recovery loop in `attempt_recovery()` reads the recipe's `max_attempts` and enforces the one-attempt policy as a guard before executing any steps:

```rust
if *attempt_count >= recipe.max_attempts {
    return RecoveryResult::EscalationRequired { reason: ... };
}
```

This means adding a new failure scenario does not require adding a new if-branch in the recovery loop. It requires adding a `FailureScenario` variant, implementing `recipe_for()` for it, and writing one test. The recovery dispatch logic never changes.

The `RecoveryStep` enum models individual actions as discriminated variants with data:

```rust
pub enum RecoveryStep {
    AcceptTrustPrompt,
    RedirectPromptToAgent,
    RebaseBranch,
    CleanBuild,
    RetryMcpHandshake { timeout: u64 },
    RestartPlugin { name: String },
    RestartWorker,
}
```

A step like `RestartPlugin { name: String }` carries its parameter directly in the enum variant. The alternative — passing a `HashMap<String, String>` or similar — loses type safety and makes it harder to reason about which steps take which parameters.

**Builder takeaway:** When a system needs to handle multiple named failure scenarios with recovery steps, encode the recipes as data (structs/enums) rather than embedding the logic in a central if/else dispatch. This creates a separable, testable surface where the recipe for each scenario can be understood in isolation.

**Evidence:** `recipe_for()` at `recovery_recipes.rs` lines 135–175 is a total function covering every `FailureScenario` variant. The test `each_scenario_has_a_matching_recipe` at `recovery_recipes.rs` lines 184–197 verifies that every variant returns a non-empty recipe with `max_attempts >= 1`.

## Lesson 4: One Automatic Recovery Before Escalation Is a Hard Constraint

The one-attempt policy in `recovery_recipes.rs` is not a guideline — it is enforced as a guard in `attempt_recovery()` before any recovery step runs. A second call to `attempt_recovery()` for the same scenario immediately returns `EscalationRequired` without executing any steps:

```rust
// Enforce one automatic recovery attempt before escalation.
if *attempt_count >= recipe.max_attempts {
    let result = RecoveryResult::EscalationRequired {
        reason: format!(
            "max recovery attempts ({}) exceeded for {}",
            recipe.max_attempts, scenario
        ),
    };
    ctx.events.push(RecoveryEvent::RecoveryAttempted { scenario, recipe, result });
    ctx.events.push(RecoveryEvent::Escalated);
    return result;
}
```

This is the ROADMAP.md principle 3 ("Recovery before escalation") encoded as a policy constraint rather than a comment or convention. The `RecoveryContext` tracks attempt counts per scenario with a `HashMap<FailureScenario, u32>`.

The test `escalation_after_max_attempts_exceeded` at `recovery_recipes.rs` lines 211–230 verifies the boundary: the first attempt succeeds and returns `Recovered`; the second attempt immediately returns `EscalationRequired`. There is no ambiguity about what "one attempt" means.

**Builder takeaway:** When encoding automatic recovery in a system, be explicit about the attempt limit as a field in the recipe data, and enforce it as a guard before executing any step. If the limit is "one," then one and only one recovery execution happens automatically before escalation fires. Without this enforcement, recovery loops silently retry indefinitely, which is manual babysitting dressed as automation.

**Evidence:** `attempt_recovery()` at `recovery_recipes.rs` lines 180–232 implements the guard. `escalation_after_max_attempts_exceeded` at `recovery_recipes.rs` lines 211–230 verifies the two-attempt boundary.

## Lesson 5: Trust Resolution Uses Deny-First Ordering

`trust_resolver.rs` resolves trust in a fixed order: denied roots are checked first, then allowlisted roots, then the default falls through to `RequireApproval`. Denial takes precedence over allowlisting. This is a safety ordering: explicit denial is a stronger signal than explicit allowlisting.

```rust
// denied first
if let Some(matched_root) = self.config.denied.iter().find(|root| path_matches(cwd, root)) {
    return TrustDecision::Required { policy: TrustPolicy::Deny, events };
}
// then allowlisted
if self.config.allowlisted.iter().any(|root| path_matches(cwd, root)) {
    return TrustDecision::Required { policy: TrustPolicy::AutoTrust, events };
}
// default: require approval
TrustDecision::Required { policy: TrustPolicy::RequireApproval, events }
```

The denial reason is stored in the `TrustEvent::TrustDenied { reason }` variant — not dropped or logged as a side effect. Downstream consumers (lane event systems, recovery recipes) can distinguish a denied path from an unresolved one.

**Builder takeaway:** In any two-list permission system (allow/deny, grant/revoke, whitelist/blacklist), decide which list wins on conflict and enforce that ordering consistently. Exposing the conflict in the output (via `TrustDenied { reason }`) rather than silently preferring one list makes the safety boundary auditable.

**Evidence:** `TrustResolver::resolve` at `trust_resolver.rs` lines 64–103 implements the ordered resolution. The test `denied_root_takes_precedence_over_allowlist` at `trust_resolver.rs` lines 156–175 verifies that a path on both lists resolves to `Deny`.

## Lesson 6: Observability Lives in a Well-Known File, Not an HTTP Endpoint

`worker_boot.rs` emits every worker state transition to `.claw/worker-state.json` under the worker's `cwd`. This is not an HTTP endpoint — the opencode binary owns its HTTP server and claw-code cannot add routes to it. The file-based approach is the practical consequence of that constraint, but it is also architecturally sound: polling a local file has lower latency and fewer failure modes than an HTTP round-trip.

```rust
fn emit_state_file(worker: &Worker) {
    let state_dir = std::path::Path::new(&worker.cwd).join(".claw");
    // atomic write: write to .tmp, then rename
    let tmp_path = state_dir.join("worker-state.json.tmp");
    let state_path = state_dir.join("worker-state.json");
    // ...
    let _ = std::fs::write(&tmp_path, json);
    let _ = std::fs::rename(&tmp_path, &state_path);
}
```

The atomic rename prevents readers from seeing a partial write. The snapshot includes `seconds_since_update` so clawhip can detect stalled workers without computing epoch deltas.

**Builder takeaway:** When building a plugin or subprocess-based system, file-based observability is often sufficient and simpler than HTTP sidecars. The atomic-write pattern (write-to-temp-then-rename) is a reliable way to produce readable state files that never contain partial JSON.

**Evidence:** `emit_state_file()` at `worker_boot.rs` lines 352–377 implements the atomic write. `claw state` CLI command at `rust/crates/rusty-claude-cli/src/main.rs` reads the file and supports both text and JSON output.

## Lesson 7: Failure Classification Bridges Detection and Recovery

`worker_boot.rs` emits `WorkerFailureKind` variants (`TrustGate`, `PromptDelivery`, `Protocol`, `Provider`). `recovery_recipes.rs` defines `FailureScenario::from_worker_failure_kind()` which maps each `WorkerFailureKind` to the corresponding `FailureScenario` that has a recovery recipe:

```rust
impl FailureScenario {
    pub fn from_worker_failure_kind(kind: WorkerFailureKind) -> Self {
        match kind {
            WorkerFailureKind::TrustGate => Self::TrustPromptUnresolved,
            WorkerFailureKind::PromptDelivery => Self::PromptMisdelivery,
            WorkerFailureKind::Protocol => Self::McpHandshakeFailure,
            WorkerFailureKind::Provider => Self::ProviderFailure,
        }
    }
}
```

This bridge is the integration point between detection and recovery. A new `WorkerFailureKind` variant requires adding the corresponding `FailureScenario` and recipe — there is no silent gap where a failure is detected but recovery is unknown.

The `LaneFailureClass` enum in `lane_events.rs` serves a similar bridging role for coordination-level failures: it classifies the root cause of a lane blockage so retry policies and dashboards can route on the failure type without parsing error strings.

**Builder takeaway:** Design a typed bridge from every failure-detection point to the corresponding recovery handler. Do not let detection and recovery evolve independently — the taxonomy of failures should be shared and explicit, so adding a new failure class requires confronting both the detection side and the recovery side simultaneously.

**Evidence:** `FailureScenario::from_worker_failure_kind` at `recovery_recipes.rs` lines 35–47 implements the bridge. The test `worker_failure_kind_maps_to_failure_scenario` at `recovery_recipes.rs` lines 353–370 verifies all four mappings. `LaneFailureClass` at `lane_events.rs` lines 39–54 provides the coordination-level failure taxonomy.

## Lesson 8: Multi-Step Recipes Model Sequential Dependency

The `StaleBranch` recipe demonstrates how multi-step recipes encode sequential dependency:

```rust
FailureScenario::StaleBranch => RecoveryRecipe {
    scenario: *scenario,
    steps: vec![RecoveryStep::RebaseBranch, RecoveryStep::CleanBuild],
    max_attempts: 1,
    escalation_policy: EscalationPolicy::AlertHuman,
}
```

The intent is: first rebase to pick up main's commits, then verify the build is clean. A failure at step 0 (rebase fails) stops the sequence and escalates rather than running `CleanBuild` in an already-diverged state. The test `stale_branch_recipe_has_rebase_then_clean_build` at `recovery_recipes.rs` lines 288–298 verifies this ordering.

The `PartialRecovery` result shape makes this explicit:

```rust
pub enum RecoveryResult {
    Recovered { steps_taken: u32 },
    PartialRecovery { recovered: Vec<RecoveryStep>, remaining: Vec<RecoveryStep> },
    EscalationRequired { reason: String },
}
```

A caller receiving `PartialRecovery { recovered, remaining }` knows exactly which steps succeeded and which were not attempted. This enables precise human handoff — the escalation message can say "rebase succeeded, clean build did not" rather than a generic failure report.

**Builder takeaway:** When encoding multi-step recovery recipes, model the steps as an ordered vector and stop on the first failure rather than attempting subsequent steps in an invalid state. Return the split between succeeded and remaining steps so the escalation message is precise.

**Evidence:** The multi-step execution loop at `recovery_recipes.rs` lines 200–212 stops on the first simulated failure (`fail_at_step == Some(i)`) and returns `PartialRecovery` with the split. `partial_recovery_when_step_fails_midway` at `recovery_recipes.rs` lines 233–254 verifies the split result shape.

## How These Mechanisms Compose

The four mechanisms form a pipeline:

1. **`worker_boot.rs`** detects a failure via screen-text observation and classifies it as a `WorkerFailureKind`
2. **`trust_resolver.rs`** independently resolves trust prompts and emits `TrustEvent` sequences
3. **`worker_boot.rs`** maps `WorkerFailureKind` to a `LaneFailureClass` and emits a `LaneEvent`
4. **`recovery_recipes.rs`** receives the `FailureScenario`, looks up the `RecoveryRecipe`, and enforces the one-attempt policy
5. **`worker_boot.rs`** writes the updated state to `.claw/worker-state.json`

The lane event schema (`lane_events.rs`) is the serialization surface that external consumers (clawhip, orchestrators) read. The recovery recipes are the policy layer that determines what happens after a failure is detected. The worker state machine is the control plane that coordinates the entire flow.

This composition is the implementation of the coordination philosophy: "humans set direction; claws perform the labor" means workers run autonomously until a failure occurs, at which point the recovery recipe attempts one automatic resolution before escalating to the human. The human is not in the loop for every tool call — they are the escalation target when the automatic recovery fails.

## Evidence

- `references/claw-code/rust/crates/runtime/src/worker_boot.rs` — `WorkerStatus` state machine, `WorkerRegistry`, `emit_state_file()`, `WorkerFailureKind`
- `references/claw-code/rust/crates/runtime/src/trust_resolver.rs` — `TrustResolver`, `TrustPolicy`, `TrustEvent`, ordered deny/allow resolution
- `references/claw-code/rust/crates/runtime/src/lane_events.rs` — `LaneEvent`, `LaneEventName`, `LaneEventStatus`, `LaneFailureClass`
- `references/claw-code/rust/crates/runtime/src/recovery_recipes.rs` — `FailureScenario`, `RecoveryRecipe`, `RecoveryStep`, `attempt_recovery()`, `RecoveryContext`
- `references/claw-code/PHILOSOPHY.md` — "humans set direction; claws perform the labor" framing
- `references/claw-code/ROADMAP.md` — "state machine first," "events over scraped prose," "recovery before escalation" principles
