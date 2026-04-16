# SWELL Audit Recovery: Runtime Wiring, Validation, and Reliability

# SWELL Audit Recovery Mission

## Plan Overview

Repair SWELL’s audited failure mode from `plan/audit-2026-04-16/*.md`: implemented subsystems exist but are not production-reachable from the real daemon runtime. This mission restores the real end-to-end path:

`Daemon -> Orchestrator -> ExecutionController -> ValidationOrchestrator -> WorktreePool/CommitStrategy -> task result`

and hardens it with explicit guardrails, tier gates, and machine-readable progress evidence for flaky workers/validators.

## Expected Functionality

### Phase A — Guardrail Setup
- Establish `crates/swell-integration-tests/tests/full_cycle_wiring.rs` as a first-class audited guardrail
- Encode the Tier 1 wiring invariants from the audit before Tier 1 implementation begins
- Map every Tier 1 blocker to a named guardrail assertion before coding starts

### Tier 1 — Runtime Wiring Blockers
1. Thread `LlmBackend` from daemon into orchestrator
2. Construct `ExecutionController` inside `Orchestrator` and add the dispatch loop
3. Wire `ValidationOrchestrator` into the real execution path
4. Install `WorktreePool` and commit-on-success
5. Install `PostToolHookManager` on the production `ToolExecutor`
6. Enforce Tier 1 gate + progress evidence

### Tier 2 — Reliability and Correctness
1. Permission ordering + dynamic bash classifier
2. Pre-tool hooks + `TurnSummary` + compaction trigger
3. 4D token accounting + prompt caching
4. `CostGuard` enforcement
5. `TaskSpec` validation at creation
6. Exact audited 5-layer `ConfigLoader` precedence + provenance
7. Session autosave + restart-boundary resume via `SessionResume`
8. Atomic `.swell/state.json` + `swell status`
9. Enforce Tier 2 gate + cross-tier blocking

### Tier 3 — Operator Surfaces and Knowledge Depth
1. REPL mode + shared slash registry
2. Manifest-backed `/resume` tab completion
3. `swell task review`
4. `swell task diff` with per-hunk accept/reject
5. `swell task set-mode`
6. MCP degraded-startup:
   - config/runtime split
   - plugin state machine
   - failure reasons + recovery hints
   - degraded reporting
7. `swell-lsp` crate and registration
8. Tool registry layering + alias normalization + structured results
9. Embedding + retrieval depth
10. Mutation testing gate
11. Background learning workers
12. Tier 3 smoke/integration coverage
13. Final Tier 3 gate enforcement

## Environment Setup

- Working directory: `/Users/ludwigjonsson/Projects/swell`
- Mission directory: `/Users/ludwigjonsson/.factory/missions/1aa0fe5b-ccbc-45d8-8970-b90e0efc9bd1`
- Existing mission-specific worker routing:
  - `runtime-wiring-worker`
  - `reliability-worker`
  - `gatekeeper-worker`

## Infrastructure and Boundaries

### Required surfaces
- Rust workspace crates in `crates/`
- Guardrail integration test target:
  - `cargo test -p swell-integration-tests --test full_cycle_wiring`

### Boundaries
- Scope is limited strictly to `plan/audit-2026-04-16/*.md`
- No extra features outside audit scope
- No mock-only replacement for wiring validation
- No Tier 2 work before Tier 1 gate is green
- No Tier 3 work before Tier 2 gate is green

## Testing and Validation Strategy

- Mission contract currently contains **53 explicit assertions**
- Every assertion is claimed exactly once by a feature
- Validation is structured around:
  - Phase A guardrail checks
  - Tier 1 wiring assertions
  - Tier 2 reliability/negative-path assertions
  - Tier 3 operator/depth assertions
  - smoke/integration coverage for Tier 3 surfaces
  - machine-readable progress and gate evidence

## Non-Functional Requirements

- Strong anti-drift protection for flaky workers
- Explicit production-caller naming
- Witness/ignore hygiene preserved
- Machine-readable checkbacks with artifact links
- No success claims on crate-local testing alone for runtime work