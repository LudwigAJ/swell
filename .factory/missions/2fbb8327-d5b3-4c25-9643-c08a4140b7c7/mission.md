# SWELL Structural Refactors: Compile-Time Wiring Safety

## Mission Overview

Design and execute a tightly scoped structural-refactor mission that hardens SWELL against the worker-pool failure mode seen in the stopped predecessor mission: production-relevant subsystems were implemented incrementally, but the worker pool was prone to leaving reachability and invariant-enforcement partially wired, especially when features were too broad or when validator tasks drifted into side work.

This mission does **not** resume or replace the suspended audit-recovery mission. It creates a strict precursor mission that lands the structural refactors described in `plan/structural-refactors/` so that the later audit-recovery work lands into safer compile-time and startup contracts.

Mission title must remain:

**SWELL Structural Refactors: Compile-Time Wiring Safety**

## Source of Truth

Read and obey only these planning sources for scope and acceptance:

- `plan/structural-refactors/00_README.md`
- `plan/structural-refactors/01_orchestrator_constructor/00_README.md`
- `plan/structural-refactors/01_orchestrator_constructor/01_problem_statement.md`
- `plan/structural-refactors/01_orchestrator_constructor/02_target_architecture.md`
- `plan/structural-refactors/01_orchestrator_constructor/03_migration_plan.md`
- `plan/structural-refactors/01_orchestrator_constructor/04_test_redundancy_audit.md`
- `plan/structural-refactors/01_orchestrator_constructor/05_acceptance_criteria.md`
- `plan/structural-refactors/02_startup_wiring_manifest.md`
- `plan/structural-refactors/03_newtype_ids.md`
- `plan/structural-refactors/04_daemon_error_enum.md`

## Non-Negotiable Scope Boundaries

- Do **not** implement any Tier 1/Tier 2/Tier 3 feature from `plan/audit-2026-04-16/*.md`.
- The **only** allowed edit under `plan/audit-2026-04-16/` is the Phase C handoff edit to `04_tier1_blockers.md` described in `plan/structural-refactors/00_README.md`.
- Do **not** begin audit-recovery work beyond that handoff. The stopped predecessor mission resumes only after this mission’s Phase C gate passes.
- Keep features single-PR, commit-shaped, and bounded to explicitly named files. “Find the right file” instructions are forbidden for this worker pool.
- Do not create validator instructions that require code review judgment. Every validator assertion must be an ordered `command -> expected result -> fail reason` check.

## Lessons Imported from the Stopped Predecessor Mission

The predecessor mission established the worker-pool operating style we must preserve:

- Every validation assertion must be claimed by exactly one feature.
- Feature ownership must be explicit and small; features that combined too many behaviors caused drift.
- Worker routing names are stable and reused here where roles still match:
  - `runtime-wiring-worker`
  - `reliability-worker`
  - `gatekeeper-worker`
- Resume/retry instability happened repeatedly. Every feature here must therefore be idempotent, self-contained, and readable after a context reset.
- Validators must not drift into opportunistic repo cleanup. They only gather mechanical pass/fail evidence and return actionable failure reasons.

## Phase Structure

### Phase A — Refactor 01: Orchestrator single constructor

This phase follows `01_orchestrator_constructor/03_migration_plan.md` **exactly**.

Required sub-sequence:
1. Phase 0 inventory
2. Phase 1 introduce `OrchestratorBuilder` behind `test-support`
3. Phase 2 migrate tests onto builder
4. Phase 3 flip the production constructor
5. Phase 4 gate the builder from production
6. Phase 5 codify future constructor policy / handoff for later runtime wiring work

Phase A gate passes only when:
- all migration phases above have landed,
- every checkbox in `01_orchestrator_constructor/05_acceptance_criteria.md` is satisfied,
- the phase-4 production-build/builder-gating checks pass,
- all anti-pattern assertions listed in this mission’s validation contract are green.

### Phase B — Refactors 02, 03, 04 in parallel

No ordering constraint exists between the three refactors, but each refactor may have internal sequencing.

- Refactor 02: startup wiring manifest
- Refactor 03: newtype IDs
- Refactor 04: daemon error enum

Phase B gate passes only when every Phase B feature is green and every Phase B assertion is green.

### Phase C — Mission completion audit

Run the four greps from `plan/structural-refactors/00_README.md#measuring-whether-its-working`, capture before/after counts in the validation artifact, and apply the cross-mission handoff edit to `plan/audit-2026-04-16/04_tier1_blockers.md`:

Replace the precondition banner with a one-line note of the form:

`Refactor 01 landed at commit <sha>; new subsystems land as required constructor parameters.`

Phase C gate passes only when:
- all four grep targets are at or below their documented targets,
- the handoff edit is committed,
- no earlier phase gate regressed.

## Worker-Facing Feature Design Rules

Every feature in `features.json` must contain two spec blocks:

- `worker_spec`
- `validator_spec`

### Worker spec requirements

Each worker spec must satisfy all of the following:

1. Single-PR scope.
2. Explicit file list; every editable file path is named.
3. Atomic commit-shaped target state.
4. Idempotent target-state description so a restarted worker can resume safely.
5. Exactly one rollback command.
6. Small enough for repeated re-reading by a flaky worker.
7. Self-contained; source-plan links are references, not prerequisites.

### Validator spec requirements

Every validator spec must be an ordered list of mechanical checks. Each step must include:

- `cmd`
- `expect`
- `fail_reason`

Validator rules:
- No judgment calls.
- No hidden setup state.
- No network or wall-clock assertions.
- Short-circuit on first failure.
- Cheap structural checks before expensive tests.
- Every assertion is owned by exactly one feature.

## Retry / Escalation Policy

- Maximum **3 worker attempts per feature** before escalation to human review.
- Validator fail reasons are load-bearing and must be written so retries can be routed intelligently.
- Cross-phase dispatch is forbidden:
  - no Phase B before Phase A gate passes,
  - no Phase C before Phase B gate passes.

## Handoff Artifact Policy

Each completed feature must write a one-line handoff note to:

`.factory/missions/2fbb8327-d5b3-4c25-9643-c08a4140b7c7/handoffs/<feature-id>.md`

The note must include:
- what landed,
- commit SHA,
- which validation assertions now pass.

For `A-01-phase0-constructor-inventory`, the handoff file itself is also the inventory artifact; this is the single allowed exception to the usual one-line handoff-note pattern.

These handoffs exist to help restarted workers warm-start without reading the full mission history.

## Worker Routing Policy

Use these routing names unless a feature explicitly justifies an exception:

- `runtime-wiring-worker`
  - Phase A implementation features that touch orchestrator/daemon constructor wiring
  - all Refactor 02 features
- `gatekeeper-worker`
  - Phase A inventory, anti-pattern gate, and final audit-style enforcement features
  - all Phase C work
- `structural-refactor-worker`
  - all Refactor 03 features
- `reliability-worker`
  - all Refactor 04 features

## Why this mission is ready for this worker pool

This mission deliberately avoids the predecessor mission’s main failure modes:
- no oversized compound features,
- no validator tasks that invite repo-wide cleanup,
- no assertion ownership ambiguity,
- no “find the right file” discovery work inside implementation features,
- no cross-phase leakage.
