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
## Area: Phase A Guardrail Setup

### VAL-PHASEA-001: full_cycle_wiring guardrail file exists before Tier 1 work
Acceptance: `crates/swell-integration-tests/tests/full_cycle_wiring.rs` exists before any Tier 1 implementation feature is considered complete.
Evidence: mission review plus git history or task ordering proof.
Validator: mission review and repository inspection

### VAL-PHASEA-002: Guardrail file encodes the audited Tier 1 wiring invariants
Acceptance: `full_cycle_wiring.rs` contains the documented Tier 1 invariant coverage from `07_integration_test_strategy.md` with loud invariant-oriented test naming and anti-deletion guidance.
Evidence: test file inspection plus compile/test proof.
Validator: `cargo test -p swell-integration-tests --test full_cycle_wiring`

### VAL-PHASEA-003: Every Tier 1 blocker maps to a named guardrail assertion before coding starts
Acceptance: each Tier 1 blocker is mapped to at least one named `full_cycle_wiring` assertion before implementation begins.
Evidence: mission feature mapping plus test file inspection.
Validator: mission review and test file inspection

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

### VAL-REL-005A: Session autosave persists unfinished session state
Acceptance: unfinished session state is persisted under the audited workspace fingerprint/session surfaces before shutdown.
Evidence: session persistence tests and artifact inspection.
Validator: crate-scoped tests plus integration extension

### VAL-REL-005B: Session resume works after daemon restart boundaries
Acceptance: after fresh daemon construction, unfinished session state is rediscovered by workspace fingerprint and resumed through the audited `SessionResume` path.
Evidence: restart/resume tests across daemon restart.
Validator: crate-scoped tests plus integration extension

### VAL-REL-006A: `.swell/state.json` writes atomically on task transitions
Acceptance: state file updates reflect task transitions and survive interruptions using audited temp-file-then-rename semantics without partial writes.
Evidence: filesystem/state tests.
Validator: crate-scoped tests

### VAL-REL-006B: `swell status` reads atomic `.swell/state.json` directly
Acceptance: the operator `swell status` surface reads the atomic state file and reports current task state without requiring daemon IPC.
Evidence: CLI/state tests.
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

### VAL-REL-010A: ConfigLoader applies the exact audited five-layer merge order
Acceptance: configuration is resolved from exactly the audited five precedence layers (`user-legacy`, `user-modern`, `project-legacy`, `project-modern`, `local-override`) with deterministic merge behavior.
Evidence: configuration loader tests covering exact precedence order.
Validator: crate-scoped tests

### VAL-REL-010B: ConfigLoader records per-value provenance for merged configuration
Acceptance: the configuration loader records auditable provenance for merged values in the audited load/audit trail surface.
Evidence: configuration loader tests covering provenance output.
Validator: crate-scoped tests

### VAL-TIER3-001A: REPL mode and shared slash command registry are production-reachable
Acceptance: CLI supports REPL mode and the shared slash command registry is reachable through the operator surface described by the audit.
Evidence: CLI/daemon tests proving the surface exists and is wired.
Validator: crate-scoped tests

### VAL-TIER3-001B: `/resume` tab completion is populated from session manifests
Acceptance: REPL tab completion for `/resume <session-id>` is backed by the audited session manifest surfaces.
Evidence: CLI/daemon tests plus manifest-backed completion proof.
Validator: crate-scoped tests

### VAL-TIER3-002A: Task review command is wired through the operator flow
Acceptance: `swell task review <id>` is available and invokes the intended daemon/runtime review pathway.
Evidence: CLI/daemon tests for the review command path.
Validator: crate-scoped tests

### VAL-TIER3-002B: Task diff supports per-hunk accept/reject through the operator flow
Acceptance: `swell task diff <id>` exposes the audited diff surface with per-hunk accept/reject behavior.
Evidence: CLI/daemon tests for diff rendering and hunk-level actions.
Validator: crate-scoped tests

### VAL-TIER3-002C: Task set-mode command updates autonomy mid-task
Acceptance: `swell task set-mode <id> <mode>` updates the active autonomy mode through the intended operator/runtime pathway.
Evidence: CLI/daemon tests for set-mode command behavior.
Validator: crate-scoped tests

### VAL-TIER3-003A: MCP degraded startup preserves config inspection when runtime bridge is unhealthy
Acceptance: MCP startup keeps config inspection/operator visibility available even when the live runtime bridge is degraded.
Evidence: tool/daemon tests.
Validator: crate-scoped tests

### VAL-TIER3-003B: MCP plugin state machine exposes audited degraded and failed states
Acceptance: the audited plugin state lifecycle includes degraded/failed states exposed through the runtime surfaces.
Evidence: tool/daemon tests.
Validator: crate-scoped tests

### VAL-TIER3-003C: MCP failures surface reason and recovery_hint
Acceptance: failed MCP servers surface audited `reason` and actionable `recovery_hint` information.
Evidence: tool/daemon tests.
Validator: crate-scoped tests

### VAL-TIER3-003D: MCP degraded report lists working, failed, and unavailable tools
Acceptance: degraded reporting exposes audited `working`, `failed`, and `unavailable_tools` surfaces.
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

### VAL-TIER3-SMOKE-001: Every Tier 3 operator surface has smoke/integration coverage
Acceptance: each Tier 3 feature has at least one smoke or integration proof showing the operator/runtime surface is reachable beyond crate-local unit tests.
Evidence: smoke/integration test inventory and command output.
Validator: crate-scoped plus smoke/integration tests

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

### VAL-PROGRESS-004: Checkbacks link to concrete machine-readable evidence artifacts
Acceptance: every mission checkback links the exact progress-checkback entry, validation-state assertion IDs, and relevant test target artifacts that justify the reported status.
Validator: mission progress review and artifact inspection

### VAL-PROGRESS-005: Gate transitions cite the exact artifact entries that justify status changes
Acceptance: Tier gate transitions are justified by concrete machine-readable artifact links rather than narrative-only status claims.
Validator: mission progress review and artifact inspection
