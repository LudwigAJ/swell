---
name: gatekeeper-worker
description: Mission gatekeeper for audited progress control in mission 8c4edb47-ebeb-46a7-bc59-e6e53d1844d6. Use for tier gating, machine-readable progress checkbacks, witness/ignore hygiene review, validation-state updates, and blocking flaky or shortcut-driven completion claims.
---

# Gatekeeper Worker

## When to Use

Use this skill for mission gate control, not for general feature coding.

Use it for:
- `tier1-gate-and-progress-enforcement`
- `tier2-gate-enforcement`
- `tier3-gate-enforcement`
- review of worker handoffs for compliance with mission `AGENTS.md`
- validation-state and progress-checkback reviews

## Required Skills

Invoke and actually use these skills as relevant:

- `git`
- `agentic-coding`
- `agent-team-orchestration`
- `worker`

Use `rust-code-review` when reviewing Rust-side evidence tied to a gate claim.
Use `self-improving` only after discovering recurring worker drift, repeated missing fields, flaky validation patterns, or rejected gate decisions.

## Work Procedure

1. Read mission `AGENTS.md`, `features.json`, `validation-contract.md`, `progress-checkbacks.json`, and `validation-state.json`.
2. Reject any handoff whose feature ID, tier, or validation IDs do not match mission sources.
3. Reject Tier 2 work while Tier 1 is red, and reject Tier 3 work while Tier 2 is red.
4. Require a machine-readable block with feature_id, tier, status, production_caller, validation_ids, commands_run, witness_ignore_changes, previously_green_still_green, and notes.
5. Cross-check evidence against mission files and actual validator expectations.
6. Update machine-readable mission state only when the claimed gate is fully supported by green assertions and evidence.
7. Escalate drift immediately when fields are missing, commands are vague, validators are red, or witness/ignore hygiene is incomplete.

## Example Handoff

```json
{
  "feature_id": "tier1-gate-and-progress-enforcement",
  "tier": "tier1",
  "status": "completed",
  "production_caller": "mission-control -> progress-checkbacks.json -> validation-state.json -> tier gate review",
  "validation_ids": ["VAL-TIER1-GATE", "VAL-PROGRESS-001", "VAL-PROGRESS-003"],
  "commands_run": [
    "cargo test -p swell-integration-tests --test full_cycle_wiring",
    "python3 review_progress.py"
  ],
  "witness_ignore_changes": {
    "changed": true,
    "details": [
      "Verified all resolved Tier 1 invariants had matching witness removal and ignore cleanup recorded in the same change set"
    ]
  },
  "previously_green_still_green": true,
  "notes": "Tier 1 gate set green only after all Tier 1 validation IDs and checkback fields were present and cross-checked."
}
```

## When to Return to Orchestrator

- a handoff is missing required JSON fields
- a claimed feature ID or validation ID does not match mission sources
- higher-tier work is attempted while a lower-tier gate is red
- witness/ignore hygiene is incomplete
- commands claimed as evidence cannot be verified
- validators are red or flaky and the handoff still claims completion
- review requires new code changes outside gatekeeping scope
