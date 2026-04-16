---
name: reliability-worker
description: Mission worker for Tier 2 audited reliability and negative-path work in mission 8c4edb47-ebeb-46a7-bc59-e6e53d1844d6. Use for permission enforcement, hooks, compaction, token accounting, cost guards, config/session/state correctness, and regression-proof validation that preserves Tier 1 green status.
---

# Reliability Worker

## When to Use

Use this skill only for Tier 2 reliability and correctness features in mission `8c4edb47-ebeb-46a7-bc59-e6e53d1844d6`, after `VAL-TIER1-GATE` is green.

## Required Skills

Invoke and actually use these skills as relevant:

- `rust`
- `rust-worker`
- `rust-code-review`
- `git`
- `agentic-coding`
- `worker`

Use `self-improving` only after a failed validator, flaky test pattern, rejected work, or discovered reliability drift.
Use `agent-team-orchestration` only when the reliability feature is intentionally decomposed across workers.

## Work Procedure

1. Read mission `AGENTS.md`, `features.json`, and `validation-contract.md`.
2. Confirm the target feature is Tier 2 and that Tier 1 is already green; if Tier 1 is red or unknown, return blocked immediately.
3. Map the feature to the exact audited runtime surface and negative/reliability path that must be proven.
4. Write adversarial tests first for denied, paused, misordered, or corrupted states before implementation.
5. Implement the smallest audited fix while preserving Tier 1 wiring.
6. Run affected crate tests, check, and clippy, plus relevant integration assertions where the audit requires production-path proof.
7. Reconfirm no Tier 1 regression using `full_cycle_wiring`.
8. Return only with machine-readable gate evidence naming feature ID, validation IDs, commands run, and whether Tier 1 stayed green.

## Example Handoff

```json
{
  "feature_id": "tier2-costguard-enforcement",
  "tier": "tier2",
  "status": "completed",
  "production_caller": "crates/swell-orchestrator/src/execution_controller.rs -> ExecutionController::run_turn_loop -> CostGuard::evaluate",
  "validation_ids": ["VAL-NEG-003", "VAL-REL-003"],
  "commands_run": [
    "cargo test -p swell-orchestrator costguard",
    "cargo test -p swell-integration-tests --test full_cycle_wiring budget_exceed_pauses_task",
    "cargo check -p swell-orchestrator",
    "cargo clippy -p swell-orchestrator -- -D warnings",
    "cargo test -p swell-integration-tests --test full_cycle_wiring"
  ],
  "witness_ignore_changes": {"changed": false, "details": []},
  "previously_green_still_green": true,
  "notes": "Warning threshold emits once; hard limit pauses task and blocks completion through the production loop."
}
```

## When to Return to Orchestrator

- Tier 1 gate is red, missing, or unverified
- requested feature is not an audited Tier 2 item
- negative path cannot be proven with existing or justified tests
- only way forward would weaken `full_cycle_wiring` or bypass production-path checks
- change causes any Tier 1 regression
- flaky behavior appears and cannot be removed without broad out-of-scope work
- work is actually Tier 3 scope
