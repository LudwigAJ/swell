# Handoff: A-01-phase0-constructor-inventory

## What Landed

Phase 0 inventory artifact created at:
`.factory/missions/c7bee451-70fd-412e-a241-4828bab650d9/inventory/A-01-phase0-constructor-inventory.md`

The inventory documents all call sites of:
- **`Orchestrator::new()`** — ~87 call sites (44 in test files, ~42 in source doctests, 1 in daemon commands.rs)
- **`Orchestrator::with_llm()`** — 1 production call site in `crates/swell-daemon/src/server.rs:57`
- **`Orchestrator::with_checkpoint_manager()`** — 0 call sites (can be removed without breaking anything)
- **`.llm_backend()`** — 1 call site in integration test

## Validation IDs Now Expected to Pass

- **VAL-A1-001**: Constructor-call inventory exists and names all current call sites ✅ (inventory created and committed)

## Key Findings for Next Worker

1. Production call path: `Daemon::new → Orchestrator::with_llm(llm) → ExecutionController::new`
2. Tests use `Orchestrator::new()` which leaves `llm_backend` and `execution_controller` as `None`
3. `llm_backend()` returns `Option<Arc<dyn LlmBackend>>` — the `Option` wrapper is what Phase A3 eliminates
4. `with_checkpoint_manager()` has zero call sites — can be safely deleted in Phase A3

## Commit SHA

`6b30189`
