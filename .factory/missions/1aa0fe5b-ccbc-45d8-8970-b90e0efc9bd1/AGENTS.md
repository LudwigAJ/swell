# Mission AGENTS.md — Audit Recovery: Runtime Wiring, Validation, and Reliability

## Scope

This mission applies only to `/Users/ludwigjonsson/.factory/missions/1aa0fe5b-ccbc-45d8-8970-b90e0efc9bd1`.

The only source of truth for mission requirements is:

- `plan/audit-2026-04-16/00_README.md`
- `plan/audit-2026-04-16/01_daemon_orphaned_wiring.md`
- `plan/audit-2026-04-16/02_execution_path_gaps.md`
- `plan/audit-2026-04-16/03_git_worktree_orphaned.md`
- `plan/audit-2026-04-16/04_tier1_blockers.md`
- `plan/audit-2026-04-16/05_tier2_reliability.md`
- `plan/audit-2026-04-16/06_tier3_ops_and_depth.md`
- `plan/audit-2026-04-16/07_integration_test_strategy.md`

Do not invent scope outside those documents. If a change is not directly required by those audit docs, it is out of scope unless it is the smallest necessary wiring or validation support for an audited requirement.

## Mission Objective

Repair SWELL’s audited failure mode: features were implemented, unit-tested, and left unreachable from the real daemon runtime.

The required end-state is a production-reachable path:

`Daemon -> Orchestrator -> ExecutionController -> ValidationOrchestrator -> WorktreePool/CommitStrategy -> task result`

guarded by integration tests that fail loudly when wiring regresses.

## Mandatory Skills

Use these mission-relevant skills when assigning or executing work:

- `rust` — default for all Rust design and implementation decisions
- `rust-worker` — for feature implementation, red/green loops, and crate-scoped verification
- `rust-code-review` — for review passes on Rust changes, especially wiring and async/runtime correctness
- `git` — for branch/worktree/commit hygiene and evidence gathering
- `agentic-coding` — to keep work contract-first, small-diff, and evidence-backed
- `agent-team-orchestration` — for multi-worker decomposition and explicit handoffs
- `task-development-workflow` — use selectively for task sequencing discipline, but do not let it override this mission’s audit-first gating
- `self-improving` — use only after failures, rejected work, or discovered drift patterns
- `worker` — for mission workers that need to read `features.json` and `validation-contract.md`

Do not reference irrelevant skills in handoffs. Mention the skill actually used.

## Additional Mission Discipline

- First mission work item is always the guardrail file contract in `crates/swell-integration-tests/tests/full_cycle_wiring.rs`; Tier 1 coding before that is out of policy.
- Any Tier 1 handoff must reference the pre-existing invariant name it satisfied.
- Do not bundle multiple audited subsections into one feature when they have different production callers or validators.
- For file-format reliability work, validate both the audited producer and the audited consumer surface.
- REPL completion claims must identify the backing manifest source, not just command parsing.
- Command bundles must be split when one subcommand has materially deeper operator behavior than the others.
- Tier 3 operator features are not complete on unit tests alone; require smoke/integration proof.
- When audit docs specify an exact count/order of layers, preserve it verbatim in features and assertions.

## Tier Gating Rules

### Tier 1 — Runtime Wiring Blockers
Must be completed first, in audited order:

1. Thread `LlmBackend` from daemon into orchestrator
2. Construct `ExecutionController` inside `Orchestrator` and add dispatch loop
3. Wire `ValidationOrchestrator` into `ExecutionController`
4. Install `WorktreePool` and commit-on-success
5. Install `PostToolHookManager` on the production `ToolExecutor`

### Tier 2 — Reliability and Correctness
Do not begin until the Tier 1 wiring gate is green.

Tier 2 includes only the audited items from `05_tier2_reliability.md`.

### Tier 3 — Operator Surfaces and Knowledge Depth
Do not begin until the Tier 2 reliability gate is green.

Tier 3 includes only the audited items from `06_tier3_ops_and_depth.md`.

### Hard Gate
If a lower-tier gate is red, any higher-tier work is automatically out of policy.

## Mission Boundaries

### In Scope
- Production runtime wiring explicitly called out in the audit
- Integration guardrail tests required by the audit
- Reliability mechanisms explicitly listed in Tier 2
- Tier 3 operator/depth work only after earlier tier gates are green
- Minimal supporting refactors required to make audited runtime wiring real and testable

### Out of Scope
- New features not named in the audit docs
- Cleanup-only refactors
- Cosmetic edits
- Replacing integration coverage with mock-only unit tests
- “While we’re here” work outside audited blockers/reliability/depth items

## Non-Negotiable Anti-Drift Rules

1. No orphan features. Do not implement any module without naming its production caller.
2. No unit-test-only completion claims. Crate-local green is necessary but never sufficient for runtime work.
3. No weakened wiring tests. Do not delete, soften, or replace `full_cycle_wiring` invariants with mocks.
4. No cross-tier drift. Do not start Tier 2 while Tier 1 is red. Do not start Tier 3 while Tier 2 is red.
5. No audit drift. If a task cannot be mapped to an audit requirement, stop considering it mission scope.
6. No fake success. Validation failure, denied tool execution, or budget exhaustion must block success exactly as audited.
7. No silent witness rot. If a fix invalidates a witness or ignored broken-state assumption, update the matching invariant hygiene in the same change.

## Witness / Ignore Hygiene

The repository audit rules are mandatory for this mission:

- Do not delete tests from `crates/swell-integration-tests/tests/full_cycle_wiring.rs`
- Do not convert wiring tests into mock-only tests
- If you complete a blocker tied to an ignored wiring invariant:
  - remove the matching `#[ignore]`
  - delete the matching `witness_*` test
  - keep both changes in the same change/commit
- If a witness starts failing because the real fix landed, that is a signal to un-ignore the corresponding invariant immediately
- If compilation changes break a wiring invariant test, update it; do not remove it

Any handoff that changes ignore/witness state must explicitly say so.

## Required Machine-Readable Progress Checkbacks

Every worker handoff must include all of the following fields in a machine-readable block:

```json
{
  "feature_id": "tier1-thread-llm-backend-from-daemon",
  "tier": "tier1",
  "status": "completed|blocked|failed",
  "production_caller": "crates/swell-daemon/src/main.rs -> Daemon::new -> Orchestrator::new",
  "validation_ids": ["VAL-WIRING-001", "VAL-WIRING-002"],
  "commands_run": [
    "cargo test -p swell-integration-tests --test full_cycle_wiring",
    "cargo test -p swell-daemon",
    "cargo test -p swell-orchestrator"
  ],
  "witness_ignore_changes": {
    "changed": false,
    "details": []
  },
  "artifact_links": {
    "progress_checkback_path": "/Users/ludwigjonsson/.factory/missions/1aa0fe5b-ccbc-45d8-8970-b90e0efc9bd1/progress-checkbacks.json",
    "validation_state_path": "/Users/ludwigjonsson/.factory/missions/1aa0fe5b-ccbc-45d8-8970-b90e0efc9bd1/validation-state.json",
    "assertion_ids": ["VAL-WIRING-001"],
    "test_targets": ["crates/swell-integration-tests/tests/full_cycle_wiring.rs"]
  },
  "previously_green_still_green": true,
  "notes": "short factual note only"
}
```

### Field requirements
- `feature_id`: must match mission `features.json`
- `tier`: `tier1`, `tier2`, or `tier3`
- `status`: `completed`, `blocked`, or `failed`
- `production_caller`: exact runtime caller path; never omit for runtime changes
- `validation_ids`: must cite mission validation IDs from `validation-contract.md`
- `commands_run`: exact commands actually executed
- `witness_ignore_changes`: must say whether any witness/ignore state changed
- `artifact_links`: exact paths/IDs tying the checkback to progress, validation state, and test targets
- `previously_green_still_green`: required for every checkback after the first green gate
- `notes`: terse factual summary only

Any handoff missing this block is incomplete.

## Required Validation Discipline

### For Tier 1
After every Tier 1 feature:
- run the relevant `full_cycle_wiring` assertion(s)
- run affected crate tests
- record which validation IDs are now green
- confirm previously-green wiring assertions stayed green

Tier 1 is not complete until `VAL-TIER1-GATE` is green.

### For Tier 2
After every Tier 2 feature:
- run crate-scoped validators for touched crates
- run the relevant negative/reliability tests
- confirm no Tier 1 wiring regression

Tier 2 is not complete until `VAL-TIER2-GATE` is green.

### For Tier 3
After every Tier 3 feature:
- run affected crate validators
- confirm Tier 1 and Tier 2 gates remain green

### Validator hierarchy
1. `cargo test -p swell-integration-tests --test full_cycle_wiring` for wiring invariants
2. affected crate `cargo test`
3. affected crate `cargo check`
4. affected crate `cargo clippy -- -D warnings`
5. broader validation only when the audited change crosses crate boundaries enough to require it

Do not report success with failing validators.

## Required Execution Order

### Phase A — Guardrail Setup
First ensure `crates/swell-integration-tests/tests/full_cycle_wiring.rs` exists and encodes the audited wiring invariants from `07_integration_test_strategy.md`.

### Phase B — Tier 1
Implement audited runtime blockers one by one, with verification after each.

### Phase C — Tier 2
Only after Tier 1 gate is green, implement audited reliability features and their negative-path proofs.

### Phase D — Tier 3
Only after Tier 2 gate is green, implement audited operator/depth features.

## Completion Standard

A feature is complete only if all are true:

- it matches an audited requirement
- its production caller is named
- required validators ran
- required validation IDs are cited
- relevant wiring/reliability tests are green
- any witness/ignore hygiene was handled in the same change
- previously-green gates stayed green

If any of those are missing, the feature is incomplete.

## Stop Conditions

Stop and report blocked instead of drifting if:

- no production caller can be identified
- crate tests pass but `full_cycle_wiring` is red
- a witness test flips but the matching invariant hygiene is not updated
- the requested work belongs to a later tier while an earlier tier gate is red
- the change needed is not justified by `plan/audit-2026-04-16/*.md`
