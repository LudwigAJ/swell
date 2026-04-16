# SWELL Audit Recovery Validation Contract

This validation contract is derived exclusively from `plan/audit-2026-04-16/*.md`, especially `04_tier1_blockers.md`, `05_tier2_reliability.md`, and `07_integration_test_strategy.md`.

The purpose of this contract is to prevent the audited failure mode: code that compiles, passes unit tests, and is never reached from the production runtime.

---

## Validation Rules

1. No Tier 1 feature is complete without a matching wiring assertion in `full_cycle_wiring.rs`.
2. No Tier 2 feature is complete without proving the relevant negative or reliability path.
3. Unit tests are necessary but insufficient.
4. Wiring tests must use the real daemon/orchestrator/execution path, with a deterministic scenario mock LLM only where external LLM calls would otherwise make the test non-deterministic.
5. Do not delete or weaken wiring guardrail tests.

---

## Area: Tier 1 Wiring Guardrails

### VAL-WIRING-001: Daemon accepts injected LLM dependency
Acceptance: `Daemon::new` accepts an `Arc<dyn LlmBackend>` or equivalent execution dependency surface.
Evidence: integration test fails to compile if daemon remains disconnected.
Validator: `cargo test -p swell-integration-tests --test full_cycle_wiring`

### VAL-WIRING-002: Orchestrator holds injected LLM and production execution controller
Acceptance: `Orchestrator` exposes the production `ExecutionController`, and identity checks prove it uses the same injected LLM instance.
Evidence: `Arc::ptr_eq`-style assertion in integration test.
Validator: `cargo test -p swell-integration-tests --test full_cycle_wiring`

### VAL-WIRING-003: Worktree pool is production-reachable
Acceptance: `Orchestrator` exposes a real `WorktreePool`, and allocation yields a path on disk.
Evidence: integration test allocates and verifies path existence.
Validator: `cargo test -p swell-integration-tests --test full_cycle_wiring`

### VAL-WIRING-004: Validation orchestrator is wired into execution path
Acceptance: `ExecutionController` and `Orchestrator` use the same `ValidationOrchestrator` instance in production.
Evidence: identity or observable call-path assertion in integration test.
Validator: `cargo test -p swell-integration-tests --test full_cycle_wiring`

### VAL-WIRING-005: Production tool executor has post-tool hook manager installed
Acceptance: the tool executor used by the real execution path has a hook manager.
Evidence: integration test asserts hook manager presence via accessor and/or fired test hook.
Validator: `cargo test -p swell-integration-tests --test full_cycle_wiring`

---

## Area: Full-Cycle Runtime Assertions

### VAL-CYCLE-001: Task reaches Done through real daemon path
Acceptance: task submitted through command handling reaches `TaskState::Done` within the configured test timeout.
Evidence: full-cycle scenario using deterministic mock LLM.
Validator: `cargo test -p swell-integration-tests --test full_cycle_wiring`

### VAL-CYCLE-002: Planner, generator, evaluator loop actually ran
Acceptance: scripted mock LLM records the expected number of calls for planning, generation, and evaluation.
Evidence: call count matches audited scenario.
Validator: `cargo test -p swell-integration-tests --test full_cycle_wiring`

### VAL-CYCLE-003: Generated file lands inside allocated task worktree
Acceptance: generated artifact exists under the allocated task worktree, not the repo root.
Evidence: filesystem assertion in integration test.
Validator: `cargo test -p swell-integration-tests --test full_cycle_wiring`

### VAL-CYCLE-004: Successful validated task creates git branch and commit trailer
Acceptance: branch `swell/<task-id>` exists and HEAD commit contains `Task-Id: <task-id>`.
Evidence: git artifact assertion in integration test.
Validator: `cargo test -p swell-integration-tests --test full_cycle_wiring`

### VAL-CYCLE-005: Validation result is recorded in task checkpoint or task result state
Acceptance: validation output from `ValidationOrchestrator` is persisted to the task’s runtime state.
Evidence: integration test verifies recorded `TaskValidationResult`.
Validator: `cargo test -p swell-integration-tests --test full_cycle_wiring`

### VAL-CYCLE-006: Post-tool hooks actually fire in production path
Acceptance: installed test hook fires at least once during scenario execution.
Evidence: counter/assertion in integration test.
Validator: `cargo test -p swell-integration-tests --test full_cycle_wiring`

---

## Area: Negative and Reliability Paths

### VAL-NEG-001: Pre-tool hook denial fails task cleanly
Acceptance: denied operation transitions task to `Failed`, with denial surfaced in transcript/result.
Source: `07_integration_test_strategy.md`, Tier 2 dependency on pre-tool hooks.
Validator: `cargo test -p swell-integration-tests --test full_cycle_wiring`

### VAL-NEG-002: Validation failure blocks success and blocks commit
Acceptance: when validation fails, task transitions to `Failed`, no success is emitted, and no git commit is produced.
Source: `04_tier1_blockers.md`, `07_integration_test_strategy.md`.
Validator: `cargo test -p swell-integration-tests --test full_cycle_wiring`

### VAL-NEG-003: Budget exceed pauses task
Acceptance: when a scripted scenario exceeds a small budget, task transitions to `Paused` with budget failure classification.
Source: `05_tier2_reliability.md`.
Validator: `cargo test -p swell-integration-tests --test full_cycle_wiring`

### VAL-REL-001: Compaction trigger is exercised
Acceptance: high cumulative token usage triggers compaction in the execution loop.
Evidence: integration or focused runtime test with compaction observable.
Validator: crate-scoped tests plus integration extension

### VAL-REL-002: 4D token accounting is surfaced correctly
Acceptance: runtime reports input, output, cache creation, and cache read token dimensions.
Evidence: validation/task result assertions.
Validator: crate-scoped tests plus integration extension

### VAL-REL-003: Cost warning and hard pause thresholds are enforced
Acceptance: warning emitted at threshold, pause at limit.
Evidence: runtime event/state assertions.
Validator: crate-scoped tests plus integration extension

### VAL-REL-004: TaskSpec is validated before task insertion
Acceptance: invalid task specs are rejected before entering the pipeline.
Evidence: creation-path tests.
Validator: crate-scoped orchestrator tests

### VAL-REL-005: Session resume works across restart boundaries
Acceptance: unfinished session is discoverable and resumable by workspace fingerprint.
Evidence: restart/resume tests.
Validator: crate-scoped tests plus integration extension

### VAL-REL-006: `.swell/state.json` writes atomically on task transitions
Acceptance: state file updates reflect task transitions and survive interruptions without partial writes.
Evidence: filesystem/state tests.
Validator: crate-scoped tests


### VAL-REL-007: Permission ordering is enforced against required tool permissions
Acceptance: runtime tool execution compares active permission mode against each tool's required permission and blocks or allows accordingly.
Evidence: crate-scoped tests and runtime-path verification where applicable.
Validator: crate-scoped tests plus integration extension

### VAL-REL-008: Dynamic bash classification maps commands to audited risk tiers
Acceptance: bash commands are classified into audited permission levels based on command behavior rather than treated uniformly.
Evidence: classifier tests and runtime-path verification where applicable.
Validator: crate-scoped tests

### VAL-REL-009: TurnSummary events are emitted from the real execution loop
Acceptance: each execution iteration emits a structured TurnSummary event from the production runtime path.
Evidence: runtime event assertions in crate-scoped or integration tests.
Validator: crate-scoped tests plus integration extension

### VAL-REL-010: ConfigLoader applies the audited layered merge semantics
Acceptance: configuration is resolved from the audited precedence layers with deterministic merge behavior and auditable provenance.
Evidence: configuration loader tests covering override and merge behavior.
Validator: crate-scoped tests

### VAL-TIER3-001: REPL mode and slash command registry are production-reachable
Acceptance: CLI supports REPL mode and the slash command registry is reachable through the operator surface described by the audit.
Evidence: CLI/daemon tests proving the surface exists and is wired.
Validator: crate-scoped tests

### VAL-TIER3-002: Task review, diff, and set-mode CLI commands are wired through the operator flow
Acceptance: the audited operator commands are available and invoke the intended daemon/runtime pathways.
Evidence: CLI/daemon tests for each command path.
Validator: crate-scoped tests

### VAL-TIER3-003: MCP degraded-startup state machine surfaces config/runtime separation and recovery hints
Acceptance: MCP startup exposes the audited plugin states, degraded reporting, and recovery-hint behavior.
Evidence: tool/daemon tests.
Validator: crate-scoped tests

### VAL-TIER3-004: swell-lsp crate exposes audited LSP-backed tools through the registry model
Acceptance: the new crate and its diagnostics/definition/hover tools are registered and reachable through the intended runtime surface.
Evidence: crate tests and registry wiring checks.
Validator: crate-scoped tests

### VAL-TIER3-005: Tool registry layering and alias normalization preserve provenance and canonical resolution
Acceptance: registry lookups report layer provenance, normalize aliases, and expose structured result content rather than text-only output.
Evidence: core/tool registry tests.
Validator: crate-scoped tests

### VAL-TIER3-006: Retrieval-depth pipeline assembles audited embedding, index, graph, and working-memory components
Acceptance: the audited retrieval stack is present and can assemble bounded working memory from its retrieval sources.
Evidence: memory/orchestrator tests.
Validator: crate-scoped tests

### VAL-TIER3-007: Mutation testing is integrated into ValidationOrchestrator as an audited gate
Acceptance: validation can execute mutation testing post-pass and feed results into follow-up testing behavior.
Evidence: validation tests.
Validator: crate-scoped tests

### VAL-TIER3-008: Background learning workers spawn on task closure and persist audited outputs
Acceptance: task closure triggers the audited background learning workers and their outputs persist into intended learning surfaces.
Evidence: memory/orchestrator tests.
Validator: crate-scoped tests

---

## Area: Tier Completion Gates

### VAL-TIER1-GATE: Runtime wiring gate
Acceptance: all Tier 1 wiring and full-cycle assertions are green before any Tier 2 work is considered complete.
Validator: `cargo test -p swell-integration-tests --test full_cycle_wiring`

### VAL-TIER2-GATE: Reliability gate
Acceptance: Tier 2 reliability assertions are green and extend the wiring suite rather than bypassing it.
Validator: crate-scoped tests plus updated integration suite

### VAL-TIER3-GATE: Product depth gate
Acceptance: Tier 3 features are only accepted once Tier 1 and Tier 2 gates are already green.
Validator: mission review against prior gates

---

## Required Validation Behavior For Agents

- Every completed blocker must cite the specific validation ID(s) it fulfills.
- Every runtime-affecting change must name the production caller that reaches it.
- If a feature fix removes the broken-state assumption behind a witness test, the same change must un-ignore the real invariant and remove the witness.
- A green unit test suite does not override a red wiring suite.


---

## Area: Mission Progress Checkbacks

### VAL-PROGRESS-001: Tier 1 checkback after every blocker
Acceptance: every Tier 1 blocker reports the production caller, validation IDs, commands run, and whether previously-green wiring tests stayed green.
Validator: mission progress review against feature handoff notes

### VAL-PROGRESS-002: Tier gates block later-tier work
Acceptance: Tier 2 work does not proceed while `VAL-TIER1-GATE` is red, and Tier 3 work does not proceed while `VAL-TIER2-GATE` is red.
Validator: mission progress review and command history

### VAL-PROGRESS-003: Witness and ignore hygiene is enforced during progress
Acceptance: when a fix invalidates a witness or ignored blocker state, the same change removes the witness and un-ignores the matching invariant.
Validator: mission progress review plus git diff inspection
