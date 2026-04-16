# Constructor Inventory — Phase 0 (A-01-phase0-constructor-inventory)

This document inventories all call sites of `Orchestrator` constructors and accessors
discovered by the Phase 0 sweep.

## Call Sites

### `Orchestrator::new()` — parameterless constructor

#### In test files

| File | Line | Context |
|------|------|----------|
| `crates/swell-orchestrator/tests/cross_permission_in_execution.rs` | 109 | `let orchestrator = Arc::new(Orchestrator::new());` |
| `crates/swell-orchestrator/tests/cross_permission_in_execution.rs` | 207 | `let orchestrator = Arc::new(Orchestrator::new());` |
| `crates/swell-orchestrator/tests/cross_permission_in_execution.rs` | 302 | `let orchestrator = Arc::new(Orchestrator::new());` |
| `crates/swell-orchestrator/tests/cross_memory_integration.rs` | 390 | `let orchestrator = Arc::new(Orchestrator::new());` |
| `crates/swell-orchestrator/tests/uncertainty_pauses.rs` | 189 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/uncertainty_pauses.rs` | 331 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/task_interruption.rs` | 53 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/task_interruption.rs` | 78 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/task_interruption.rs` | 100 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/task_interruption.rs` | 124 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/task_interruption.rs` | 145 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/task_interruption.rs` | 181 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/task_interruption.rs` | 209 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/task_interruption.rs` | 233 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/task_interruption.rs` | 348 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/task_interruption.rs` | 378 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/task_interruption.rs` | 412 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/task_interruption.rs` | 428 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/task_interruption.rs` | 440 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/cross_config_cascade.rs` | 223 | `let orchestrator = Arc::new(Orchestrator::new());` |
| `crates/swell-orchestrator/tests/cross_config_cascade.rs` | 356 | `let orchestrator = Arc::new(Orchestrator::new());` |
| `crates/swell-orchestrator/tests/cross_full_execution_path.rs` | 89 | `let orchestrator = Arc::new(Orchestrator::new());` |
| `crates/swell-orchestrator/tests/cross_full_execution_path.rs` | 150 | `let orchestrator = Arc::new(Orchestrator::new());` |
| `crates/swell-orchestrator/tests/cross_full_execution_path.rs` | 203 | `let orchestrator = Arc::new(Orchestrator::new());` |
| `crates/swell-orchestrator/tests/cross_full_execution_path.rs` | 286 | `let orchestrator = Arc::new(Orchestrator::new());` |
| `crates/swell-orchestrator/tests/cross_recovery_flow.rs` | 509 | `let orchestrator = Arc::new(Orchestrator::new());` |
| `crates/swell-orchestrator/tests/cross_recovery_flow.rs` | 608 | `let orchestrator = Arc::new(Orchestrator::new());` |
| `crates/swell-orchestrator/tests/cross_recovery_flow.rs` | 700 | `let orchestrator = Arc::new(Orchestrator::new());` |
| `crates/swell-orchestrator/tests/cross_recovery_flow.rs` | 959 | `let orchestrator = Arc::new(Orchestrator::new());` |
| `crates/swell-orchestrator/tests/cross_recovery_flow.rs` | 1023 | `let orchestrator = Arc::new(Orchestrator::new());` |
| `crates/swell-orchestrator/tests/cross_recovery_flow.rs` | 1092 | `let orchestrator = Arc::new(Orchestrator::new());` |
| `crates/swell-orchestrator/tests/drift_detection.rs` | 15 | `let orchestrator = Arc::new(swell_orchestrator::Orchestrator::new());` |
| `crates/swell-orchestrator/tests/drift_detection.rs` | 284 | `let orchestrator = Arc::new(swell_orchestrator::Orchestrator::new());` |
| `crates/swell-orchestrator/tests/cross_cost_tracking.rs` | 47 | `let orchestrator = Arc::new(Orchestrator::new());` |
| `crates/swell-orchestrator/tests/cross_cost_tracking.rs` | 174 | `let orchestrator = Arc::new(Orchestrator::new());` |
| `crates/swell-orchestrator/tests/interactive_approval.rs` | 36 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/interactive_approval.rs` | 71 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/interactive_approval.rs` | 99 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/interactive_approval.rs` | 135 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/interactive_approval.rs` | 175 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/interactive_approval.rs` | 223 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/interactive_approval.rs` | 256 | `let orchestrator = Orchestrator::new();` |
| `crates/swell-orchestrator/tests/interactive_approval.rs` | 323 | `let orchestrator = Orchestrator::new();` |

#### In source files (doctests / internal use)

| File | Line | Context |
|------|------|----------|
| `crates/swell-orchestrator/src/feature_leads.rs` | 557 | doctest |
| `crates/swell-orchestrator/src/feature_leads.rs` | 581 | doctest |
| `crates/swell-orchestrator/src/feature_leads.rs` | 609 | doctest |
| `crates/swell-orchestrator/src/feature_leads.rs` | 646 | doctest |
| `crates/swell-orchestrator/src/feature_leads.rs` | 666 | doctest |
| `crates/swell-orchestrator/src/feature_leads.rs` | 686 | doctest |
| `crates/swell-orchestrator/src/feature_leads.rs` | 805 | doctest |
| `crates/swell-orchestrator/src/feature_leads.rs` | 831 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 1521 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 1535 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 1553 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 1567 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 1579 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 1590 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 1604 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 1619 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 1650 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 1676 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 1709 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 1727 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 1756 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 1775 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 1793 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 1809 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 1828 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 1844 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 1875 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 1906 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 2004 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 2072 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 2080 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 2105 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 2124 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 2146 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 2169 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 2209 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 2241 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 2273 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 2302 | doctest |
| `crates/swell-orchestrator/src/lib.rs` | 2331 | doctest |
| `crates/swell-orchestrator/src/execution.rs` | 1607 | doctest |
| `crates/swell-orchestrator/src/execution.rs` | 1616 | doctest |
| `crates/swell-orchestrator/src/execution.rs` | 1686 | doctest |
| `crates/swell-orchestrator/src/execution.rs` | 1702 | doctest |
| `crates/swell-orchestrator/src/execution.rs` | 1715 | doctest |
| `crates/swell-orchestrator/src/execution.rs` | 1752 | doctest |
| `crates/swell-orchestrator/src/execution.rs` | 1781 | doctest |
| `crates/swell-orchestrator/src/execution.rs` | 1812 | doctest |
| `crates/swell-orchestrator/src/execution.rs` | 1838 | doctest |
| `crates/swell-orchestrator/src/execution.rs` | 2215 | doctest |
| `crates/swell-orchestrator/src/execution.rs` | 2268 | doctest |
| `crates/swell-orchestrator/src/execution.rs` | 2317 | doctest |
| `crates/swell-orchestrator/src/execution.rs` | 2359 | doctest |
| `crates/swell-orchestrator/src/execution.rs` | 2404 | doctest |
| `crates/swell-orchestrator/src/execution.rs` | 2453 | doctest |
| `crates/swell-orchestrator/src/execution.rs` | 2493 | doctest |
| `crates/swell-daemon/src/commands.rs` | 704 | `Arc::new(Mutex::new(Orchestrator::new()))` |

#### In AGENTS.md (documentation only)

| File | Line | Context |
|------|------|----------|
| `crates/swell-orchestrator/AGENTS.md` | 249 | code example |
| `crates/swell-orchestrator/AGENTS.md` | 256 | code example |

**Total `Orchestrator::new()` call sites: ~87** (44 in test files, ~42 in source doctests, 1 in daemon commands.rs)

---

### `Orchestrator::with_llm()` — LLM-injection constructor

| File | Line | Context |
|------|------|----------|
| `crates/swell-daemon/src/server.rs` | 57 | `orchestrator: Arc::new(Mutex::new(Orchestrator::with_llm(llm)))` — **production call site** |
| `crates/swell-integration-tests/tests/full_cycle_wiring.rs` | 82 | doctest comment referencing the call chain |

**Total `Orchestrator::with_llm()` call sites: 1** (production) + 1 (integration test comment)

---

### `Orchestrator::with_checkpoint_manager()` — checkpoint manager injection constructor

**No call sites found** in any workspace Rust source.

---

### `.llm_backend()` — accessor

| File | Line | Context |
|------|------|----------|
| `crates/swell-integration-tests/tests/full_cycle_wiring.rs` | 109 | `orch.lock().await.llm_backend().expect("orchestrator must hold llm")` |

**Total `.llm_backend()` call sites: 1** (in integration test)

---

## Required Subsystems (from refactor documentation)

The orchestrator requires these production subsystems to be wired via constructor parameters:

1. **LlmBackend** — injected via `Orchestrator::with_llm()`, stored as `llm_backend: Option<Arc<dyn LlmBackend>>`
2. **ExecutionController** — constructed inside `with_llm()`, stored as `execution_controller: Arc<RwLock<Option<Arc<ExecutionController>>>`
3. **CheckpointManager** — stored as `checkpoint_manager: Arc<CheckpointManager>`, can be injected via `with_checkpoint_manager()`
4. **WorktreePool** — not yet visible in current constructor surface
5. **ValidationOrchestrator** — not yet visible in current constructor surface
6. **PostToolHookManager** — not yet visible in current constructor surface

## Notes

- The production call path is: `Daemon::new → Orchestrator::with_llm(llm) → ExecutionController::new`
- Tests use `Orchestrator::new()` which leaves `llm_backend` and `execution_controller` as `None`
- `llm_backend()` returns `Option<Arc<dyn LlmBackend>>` — the `Option` is the wrapper that Phase A3 eliminates
