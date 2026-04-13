# Lane Completion Evidence

## What This Document Covers

This document explains how ClawCode detects lane completion and feeds that signal into the policy engine. It covers the completion-detection mechanism in `tools/src/lane_completion.rs`, the policy condition and action wiring in `runtime/src/policy_engine.rs`, and how both are exercised by integration tests in `runtime/tests/integration_tests.rs`. The document stays scoped to the completion-detection and policy-action surfaces only.

## Builder Lesson

The lane completion system demonstrates a **signal → condition → action** pattern: a detection function produces a structured `LaneContext` with a `completed` flag, and the `PolicyEngine` matches that flag through a `PolicyCondition::LaneCompleted` condition to fire `PolicyAction::CloseoutLane` and `PolicyAction::CleanupSession`. This cleanly separates "when is a lane done?" (detection) from "what should happen when a lane is done?" (policy), allowing the policy rules to evolve without touching the detection logic.

---

## Completion Detection (`tools/src/lane_completion.rs`)

### Role

`lane_completion.rs` bridges the gap where `LaneContext::completed` was a passive bool that nothing automatically set. The module exposes two public functions:

- `detect_lane_completion(output, test_green, has_pushed) -> Option<LaneContext>` — the detector
- `evaluate_completed_lane(context) -> Vec<PolicyAction>` — the policy executor for completed lanes

Both functions are `#[allow(dead_code)]` at the module level, meaning the runtime does not yet wire them into an automatic call site; they exist as shippable infrastructure that is ready for integration but not yet invoked from the turn loop.

### Detection Logic

```rust
pub(crate) fn detect_lane_completion(
    output: &AgentOutput,
    test_green: bool,
    has_pushed: bool,
) -> Option<LaneContext>
```

A lane is marked completed when **all** of the following are true:

| Condition | Check |
|---|---|
| No error | `output.error.is_none()` |
| Finished or Completed status | `status.eq_ignore_ascii_case("completed") \|\| finished` |
| No current blocker | `output.current_blocker.is_none()` |
| Tests are green | `test_green == true` |
| Code has been pushed | `has_pushed == true` |

If all five conditions hold, the function returns `Some(LaneContext)` with `completed = true` and `green_level = 3` (Workspace green). If any condition fails, it returns `None`, leaving the lane active.

The returned `LaneContext` also carries:
- `review_status: ReviewStatus::Approved`
- `diff_scope: DiffScope::Scoped`
- `blocker: LaneBlocker::None`
- `reconciled: false`

### Policy Evaluation for Completed Lanes

```rust
pub(crate) fn evaluate_completed_lane(context: &LaneContext) -> Vec<PolicyAction>
```

This function creates a hardcoded `PolicyEngine` with two rules:

| Rule name | Condition | Action | Priority |
|---|---|---|---|
| `closeout-completed-lane` | `LaneCompleted AND GreenAt { level: 3 }` | `CloseoutLane` | 10 |
| `cleanup-completed-session` | `LaneCompleted` | `CleanupSession` | 5 |

The engine is constructed in-process rather than loaded from config, which means the rule set is currently static. The design supports rule evolution: adding a new rule requires only constructing a new `PolicyRule` and inserting it into the engine's rule vector.

### Unit Tests

The module includes six tests in `#[cfg(test)]`:

- `detects_completion_when_all_conditions_met` — all five conditions pass → `Some(LaneContext)` with `completed = true`
- `no_completion_when_error_present` — error set → `None`
- `no_completion_when_not_finished` — status is "Running" → `None`
- `no_completion_when_tests_not_green` — `test_green = false` → `None`
- `no_completion_when_not_pushed` — `has_pushed = false` → `None`
- `evaluate_triggers_closeout_for_completed_lane` — confirms both `CloseoutLane` and `CleanupSession` fire from `evaluate_completed_lane`

---

## Policy Engine Wiring (`runtime/src/policy_engine.rs`)

### `PolicyCondition::LaneCompleted`

```rust
Self::LaneCompleted => context.completed,
```

The condition matches when `LaneContext.completed` is `true`. This is the bridge from detection output to policy decision.

### `PolicyAction::CloseoutLane` and `PolicyAction::CleanupSession`

```rust
pub enum PolicyAction {
    CloseoutLane,
    CleanupSession,
    // ... other variants
}
```

`CloseoutLane` signals that a lane has reached a terminal state and should stop accepting new work. `CleanupSession` releases session resources associated with the lane. Both are emitted as ordered actions by the policy engine when matching rules fire.

### Priority Ordering

`PolicyEngine::new` sorts rules by ascending `priority`:

```rust
rules.sort_by_key(|rule| rule.priority);
```

Rules with lower priority numbers execute first. This allows a "reconcile first, then closeout" pattern when a lane is both reconciled and completed — the higher-priority reconcile rule fires and emits its actions before the closeout rule.

### Example: Completed Lane Rule

```rust
PolicyRule::new(
    "lane-closeout",
    PolicyCondition::LaneCompleted,
    PolicyAction::Chain(vec![PolicyAction::CloseoutLane, PolicyAction::CleanupSession]),
    30,
)
```

Applied to a `LaneContext` with `completed = true`, this rule fires and emits both actions in order.

---

## Integration Tests (`runtime/tests/integration_tests.rs`)

### `stale_branch_detection_flows_into_policy_engine`

Wires `BranchFreshness::Stale` into `PolicyEngine` and verifies `MergeForward` fires when `PolicyCondition::StaleBranch` matches. This confirms the policy engine correctly connects branch-freshness signals to merge actions.

### `end_to_end_stale_lane_gets_merge_forward_action`

Simulates a full detection → context → policy → action flow:

1. Detects `BranchFreshness::Stale { commits_behind: 5, missing_fixes: ... }`
2. Builds `LaneContext` with `green_level: 3`, `branch_freshness: 5 hours`, `ReviewStatus::Approved`
3. Creates `PolicyEngine` with two rules: one for `StaleBranch + ReviewPassed` → `MergeForward` (priority 5), one for `StaleBranch` → `Notify` (priority 10)
4. Evaluates and asserts both actions fire in priority order

### `worker_provider_failure_flows_through_recovery_to_policy`

Wires `WorkerFailureKind::Provider` → `FailureScenario::ProviderFailure` → `attempt_recovery()` → post-recovery `LaneContext` → `PolicyEngine` → `MergeToDev`. This proves the recovery context feeds back into policy decisions.

### `green_contract_satisfied_allows_merge`

Tests that `GreenContract::is_satisfied_by(GreenLevel::Workspace)` returns `true`, verifying the green-level contract mechanism.

---

## Lane Event Schema (`runtime/src/lane_events.rs`)

Completion is also observable through the typed event system:

- `LaneEventName::Finished` — emitted when a lane reaches terminal completed state
- `LaneEventStatus::Completed` — the status value paired with `Finished`
- `LaneEventName::Failed` — emitted when a lane terminates with an error
- `LaneEventName::Reconciled` — emitted when a lane is reconciled without further action (e.g., branch already merged)

The event schema provides visibility into lane lifecycle independently of the policy engine — observers can subscribe to `lane.finished` without knowing about policy rules.

---

## Parity Status

**Lane completion is not a separate parity lane.** The `tools/src/lane_completion.rs` detector and `runtime/src/policy_engine.rs` policy engine are both landed code on `main` with passing unit tests and integration test coverage. The PARITY.md 9-lane checkpoint does not include a dedicated "lane completion" lane because the mechanism is treated as infrastructure (part of the policy engine) rather than a standalone feature area.

The `integration_tests.rs` file exercises the policy engine's completion-signal handling but does not include a dedicated `#[test]` for `detect_lane_completion` end-to-end from `AgentOutput` through to a policy action — that specific path remains as future integration coverage.

---

## What Is NOT Covered Here

This document does not cover:
- How the turn loop automatically invokes `detect_lane_completion` — that call site is not yet landed
- Configuration-driven policy rules — the rule set in `evaluate_completed_lane` is hardcoded
- Lane event emission from the turn loop — `lane_events.rs` defines the schema but the emission call sites are outside scope
- The full green contract mechanism (tested in `green_contract_satisfied_allows_merge`, but not documented in depth here)
