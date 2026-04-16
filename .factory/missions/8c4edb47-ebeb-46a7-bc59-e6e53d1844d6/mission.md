# SWELL Audit Recovery Mission — Runtime Wiring, Validation, and Reliability

## Mission Source of Truth

This mission is derived exclusively from the audit documents in `plan/audit-2026-04-16/`:

- `00_README.md`
- `01_daemon_orphaned_wiring.md`
- `02_execution_path_gaps.md`
- `03_git_worktree_orphaned.md`
- `04_tier1_blockers.md`
- `05_tier2_reliability.md`
- `06_tier3_ops_and_depth.md`
- `07_integration_test_strategy.md`

No planned feature in this mission should be invented outside those documents. If new work is discovered, it must be justified as required wiring or validation support for an audited feature.

## Mission Objective

Reconnect SWELL's production runtime so that the operator-facing entry path:

`Daemon -> Orchestrator -> ExecutionController -> ValidationOrchestrator -> WorktreePool/CommitStrategy -> Task result`

is real, reachable, and protected by integration-test guardrails.

The mission exists because the audit found the recurring failure mode: code was implemented, unit tested, and left orphaned from the real daemon runtime. This mission fixes that failure mode first, then builds reliability and operator surfaces on top.

## Mission Principles

1. Production wiring is the top priority.
2. Unit-tested but unreachable code is incomplete.
3. Tier 1 must be completed before Tier 2 begins.
4. Tier 2 must be completed before Tier 3 begins.
5. Every Tier 1 feature must ship with a matching wiring assertion from `07_integration_test_strategy.md`.
6. Do not weaken, delete, or replace integration wiring tests with mock-only tests.
7. If a blocker fix makes a witness test obsolete, remove the matching `#[ignore]` and witness in the same change.

## Delivery Tiers

### Tier 1 — Runtime Wiring Blockers

The minimum viable autonomous loop is not considered functional until all five audited blockers are complete.

1. Thread an `LlmBackend` from daemon into the orchestrator
2. Construct `ExecutionController` inside `Orchestrator` and add a dispatch loop
3. Wire `ValidationOrchestrator` into `ExecutionController`
4. Install `WorktreePool` and commit-on-success
5. Install `PostToolHookManager` on the production `ToolExecutor`

### Tier 2 — Reliability and Correctness

Only start after Tier 1 integration tests pass.

1. Permission mode ordering + dynamic bash classifier
2. Pre-tool hooks, `TurnSummary`, compaction trigger
3. 4D token accounting + prompt caching
4. `CostGuard` enforcement
5. `TaskSpec` validation at creation
6. 5-layer `ConfigLoader` with merge semantics
7. Session autosave / workspace fingerprint
8. Atomic `.swell/state.json`

### Tier 3 — Operator Surfaces and Knowledge Depth

Only start after Tier 2 is green.

1. REPL mode + slash command registry
2. `swell task review | diff | set-mode` CLI commands
3. MCP degraded-startup state machine
4. New `swell-lsp` crate
5. Tool registry layering + alias normalization + structured results
6. Embedding + retrieval depth
7. Mutation testing
8. Background learning workers

## Runtime Invariants This Mission Must Protect

The following invariants come directly from the audit and must remain true once implemented:

- `Daemon::new` accepts execution dependencies instead of constructing a disconnected orchestrator.
- `Orchestrator` owns the production `ExecutionController`.
- The `ExecutionController` used at runtime holds the same injected LLM instance.
- `ValidationOrchestrator` is called in the real execution path, not just in tests.
- Work is executed in an allocated task worktree, not directly in the repo root.
- Successful validated work produces git artifacts tied to the task.
- Production tool execution includes hook managers.
- Failure paths are explicit: denied tool use fails the task, failed validation blocks success, budget exhaustion pauses work.

## Required Execution Order

### Phase A — Guardrail Setup
- Create or extend `crates/swell-integration-tests/tests/full_cycle_wiring.rs`
- Encode the Tier 1 wiring assertions from `07_integration_test_strategy.md`
- Keep tests loud and explicit about wiring invariants

### Phase B — Tier 1 Blockers
- Implement audited Tier 1 features one at a time
- After each feature, run the matching integration test(s)
- Do not batch all five without intermediate verification

### Phase C — Tier 2 Reliability
- Add pre-tool enforcement, telemetry, compaction, token accounting, budget enforcement, task spec validation, config/session/state surfaces
- Extend the integration suite with the negative-path and reliability assertions called for by the audit

### Phase D — Tier 3 Product Depth
- Add operator and platform surfaces only after the autonomous loop is already trusted


## Progress Checkbacks And Guardrails During Execution

The audit requires not just strong end-state validation, but active guardrails during mission progress so agents cannot drift into building disconnected code again.

### Mandatory checkback cadence

1. Before starting Tier 1 work, confirm the `full_cycle_wiring` suite exists and encodes the audited invariants.
2. After every Tier 1 feature, run the relevant `full_cycle_wiring` assertions and record which validation IDs are now green.
3. At Tier 1 completion, stop and verify the full Tier 1 wiring gate before any Tier 2 work begins.
4. After every Tier 2 feature, run both the crate-scoped validators and the relevant negative/reliability guardrail tests.
5. At Tier 2 completion, stop and verify the reliability gate before any Tier 3 work begins.
6. During Tier 3, continue checking back after each feature that the already-green Tier 1 and Tier 2 gates remain green.

### Mandatory progress reporting content

Every feature handoff or checkback must state:
- audited feature ID/name completed
- production caller that now reaches the feature
- validation IDs exercised
- commands run
- whether any witness or ignored invariant changed
- whether previously-green wiring tests stayed green

### Stop conditions

Stop the mission and escalate rather than continuing if:
- a feature is implemented but no production caller can be named
- crate tests pass but `full_cycle_wiring` regresses
- a witness test flips red and the matching invariant is not un-ignored in the same change
- Tier 2 or Tier 3 work is being attempted while earlier tier gates are red

## Completion Criteria

The mission is complete only when:

1. All Tier 1 features are implemented and protected by wiring tests
2. Tier 1 integration suite passes
3. Tier 2 reliability features are implemented with validation proving the failure paths
4. Tier 2 integration extensions pass
5. Tier 3 features are implemented only after the above are green
6. No audited blocker remains implemented-but-unreachable

## Explicit Anti-Failure Instructions For Flaky Agents

- Do not build a module without naming its production caller.
- Do not claim a feature is complete if only crate-local tests pass.
- Do not replace integration invariants with mocks.
- Do not silently leave `#[ignore]` on a blocker test that has been fixed.
- Do not add scope beyond `plan/audit-2026-04-16/*.md` unless strictly required to wire or validate an audited feature.
- Do not mark validation complete while the audited wiring suite is red.

## Expected Outcome

After this mission, SWELL should be able to accept a task through the daemon, execute the real agent pipeline, validate the result, isolate work in a task worktree, commit validated output, and fail loudly whenever future code regresses the daemon-to-runtime wiring.
