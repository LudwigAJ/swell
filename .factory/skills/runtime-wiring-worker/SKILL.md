---
name: runtime-wiring-worker
description: Mission worker for Tier 1 runtime wiring blockers in audit mission 8c4edb47-ebeb-46a7-bc59-e6e53d1844d6. Use for daemon→orchestrator→execution→validation→worktree→hook-manager production reachability work with strict wiring proof, witness/ignore hygiene, and machine-readable checkbacks.
---

# Runtime Wiring Worker

## When to Use

Use this skill only for Tier 1 runtime wiring blockers in mission `8c4edb47-ebeb-46a7-bc59-e6e53d1844d6`, specifically audited work that makes the production path reachable:

`Daemon -> Orchestrator -> ExecutionController -> ValidationOrchestrator -> WorktreePool/CommitStrategy -> task result`

Use it for these mission features only:

- `tier1-thread-llm-backend-from-daemon`
- `tier1-construct-execution-controller-and-dispatch-loop`
- `tier1-wire-validation-orchestrator-into-execution`
- `tier1-install-worktree-pool-and-commit-on-success`
- `tier1-install-post-tool-hook-manager-in-production`

Do not use this skill for Tier 2 reliability work, Tier 3 operator surfaces, cleanup-only refactors, or any feature that cannot name its production caller before coding starts.

## Required Skills

Invoke and actually use these skills as relevant:

- `rust`
- `rust-worker`
- `rust-code-review`
- `git`
- `agentic-coding`
- `worker`

Use `agent-team-orchestration` only if the runtime wiring task is being split across multiple workers.
Use `self-improving` only after a failed attempt, rejected patch, validator failure, or discovered drift pattern.

## Work Procedure

1. Read mission `AGENTS.md`, `features.json`, and `validation-contract.md` and confirm the requested feature is a Tier 1 blocker.
2. Record the exact production caller path that must become real before editing anything.
3. Inspect `crates/swell-integration-tests/tests/full_cycle_wiring.rs` and the current runtime gap from the audit before changing code.
4. Add or tighten the relevant `full_cycle_wiring` assertion first when needed; prove the gap before the fix.
5. Implement the smallest production-reaching change; wire real constructors and runtime callers, not test-only builders.
6. If the real blocker is fixed, remove matching `#[ignore]` and delete matching `witness_*` in the same change.
7. Run validators in order: relevant `full_cycle_wiring`, affected crate `cargo test`, affected crate `cargo check`, affected crate `cargo clippy -- -D warnings`.
8. Re-run previously green Tier 1 assertions that this change could disturb.
9. Return only with machine-readable checkback evidence naming production caller, validation IDs, commands run, witness/ignore changes, and whether previously green assertions stayed green.

## Example Handoff

```json
{
  "feature_id": "tier1-wire-validation-orchestrator-into-execution",
  "tier": "tier1",
  "status": "completed",
  "production_caller": "crates/swell-daemon/src/main.rs -> Daemon::new -> Orchestrator::new -> ExecutionController::run_task -> ValidationOrchestrator::validate",
  "validation_ids": ["VAL-WIRING-004", "VAL-CYCLE-005", "VAL-NEG-002"],
  "commands_run": [
    "cargo test -p swell-integration-tests --test full_cycle_wiring validation_orchestrator",
    "cargo test -p swell-validation",
    "cargo test -p swell-orchestrator",
    "cargo check -p swell-orchestrator",
    "cargo clippy -p swell-orchestrator -- -D warnings"
  ],
  "witness_ignore_changes": {
    "changed": true,
    "details": [
      "Removed #[ignore] from wiring_validation_orchestrator_reachable",
      "Deleted witness_validation_orchestrator_still_orphaned"
    ]
  },
  "previously_green_still_green": true,
  "notes": "Runtime execution now calls the production validation orchestrator before success; validation failure still blocks commit and done state."
}
```

## When to Return to Orchestrator

- requested work is not a Tier 1 audited blocker
- no exact production caller can be named
- `full_cycle_wiring` proof is missing or would need to be weakened
- witness flipped but matching ignore/witness hygiene was not completed
- previously green Tier 1 invariant regressed
- change drifts into Tier 2 or Tier 3 work
- unrelated validator failures prevent trustworthy completion
