# Validation Contract — SWELL Structural Refactors: Compile-Time Wiring Safety

All assertions in this contract are mechanical. Every assertion is claimed by exactly one feature in `features.json`.

Assertion categories used below:
- `cargo-command`: `cargo check`, `cargo test`, `cargo build`, `cargo clippy`
- `grep-count`: `rg`-based structural assertions
- `exit-code`: command success/failure without content matching
- `human-review`: reserved only for the final handoff artifact sanity checks

## Phase A — Refactor 01: Orchestrator single constructor

### Area A1 — Inventory and builder introduction

#### VAL-A1-001: Constructor-call inventory exists and names all current call sites
Type: grep-count
Acceptance: a checked-in inventory artifact lists every call site of `Orchestrator::new`, `Orchestrator::with_llm`, `Orchestrator::with_checkpoint_manager`, and `.llm_backend()` discovered by the phase-0 sweep.

#### VAL-A1-002: Test-only builder module exists
Type: grep-count
Acceptance: `crates/swell-orchestrator/src/builder.rs` exists and defines `OrchestratorBuilder`.

#### VAL-A1-003: Builder type is test-gated
Type: grep-count
Acceptance: `OrchestratorBuilder` type definition in `builder.rs` is guarded by `#[cfg(any(test, feature = "test-support"))]`.

#### VAL-A1-004: Builder impls are test-gated
Type: grep-count
Acceptance: all `impl OrchestratorBuilder` blocks are guarded by `#[cfg(any(test, feature = "test-support"))]`.

#### VAL-A1-005: Builder export is test-gated
Type: grep-count
Acceptance: any `pub use builder::OrchestratorBuilder` export in `crates/swell-orchestrator/src/lib.rs` is guarded by `#[cfg(any(test, feature = "test-support"))]`.

#### VAL-A1-006: test-support feature declared
Type: grep-count
Acceptance: `crates/swell-orchestrator/Cargo.toml` declares `test-support = []` under `[features]`.

### Area A2 — Test migration onto builder

#### VAL-A2-001: No tests use Orchestrator::new()
Type: grep-count
Acceptance: test and integration code contains zero call sites of parameterless `Orchestrator::new()`.

#### VAL-A2-002: No tests use Orchestrator::with_llm()
Type: grep-count
Acceptance: test and integration code contains zero call sites of `Orchestrator::with_llm(...)`.

#### VAL-A2-003: No tests use Orchestrator::with_checkpoint_manager()
Type: grep-count
Acceptance: test and integration code contains zero call sites of `Orchestrator::with_checkpoint_manager(...)`.

#### VAL-A2-004: Workspace still compiles after test migration
Type: cargo-command
Acceptance: `cargo check --workspace` exits 0 after migrating tests to the builder.

### Area A3 — Production constructor flip

#### VAL-A3-001: llm_backend is non-optional
Type: grep-count
Acceptance: `crates/swell-orchestrator/src/lib.rs` contains `llm_backend: Arc<dyn ...LlmBackend>` and contains zero matches for `llm_backend: Option`.

#### VAL-A3-002: Exactly one public Orchestrator constructor remains
Type: grep-count
Acceptance: `crates/swell-orchestrator/src/lib.rs` contains exactly one public constructor matching `pub fn new(...)` / `pub async fn new(...)`, and zero public `with_*` constructors.

#### VAL-A3-003: with_llm is deleted
Type: grep-count
Acceptance: `crates/swell-orchestrator/src/lib.rs` and workspace Rust sources contain zero public definitions of `with_llm` on `Orchestrator`.

#### VAL-A3-004: with_checkpoint_manager is deleted
Type: grep-count
Acceptance: `crates/swell-orchestrator/src/lib.rs` and workspace Rust sources contain zero public definitions of `with_checkpoint_manager` on `Orchestrator`.

#### VAL-A3-005: llm_backend accessor returns Arc, not Option
Type: grep-count
Acceptance: `Orchestrator::llm_backend()` in `crates/swell-orchestrator/src/lib.rs` returns `Arc<dyn ...LlmBackend>` and does not return `Option<...>`.

#### VAL-A3-006: Daemon constructs Orchestrator via Orchestrator::new(llm)
Type: grep-count
Acceptance: `crates/swell-daemon/src/server.rs` calls `Orchestrator::new(llm)` and contains no `Orchestrator::with_llm` call.

#### VAL-A3-007: Workspace compiles after constructor flip
Type: cargo-command
Acceptance: `cargo check --workspace` exits 0 after the production constructor flip.

#### VAL-A3-008: Full workspace tests pass after constructor flip
Type: cargo-command
Acceptance: `cargo test --workspace` exits 0 after the production constructor flip.

### Area A4 — Builder gating and compile-time contract

#### VAL-A4-001: compile_fail doctest exists for parameterless construction
Type: grep-count
Acceptance: `crates/swell-orchestrator/src/lib.rs` contains a `compile_fail` doctest proving `swell_orchestrator::Orchestrator::new()` without parameters does not compile.

#### VAL-A4-002: Workspace doctests pass
Type: cargo-command
Acceptance: `cargo test --workspace --doc` exits 0.

#### VAL-A4-003: Production build excludes OrchestratorBuilder symbol
Type: exit-code
Acceptance: the phase-4 production build / symbol-grep check exits 0, proving `OrchestratorBuilder` is unreachable from production output.

### Area A5 — Anti-pattern enforcement from target architecture

#### VAL-A5-001: No Arc<Mutex<Option<...>>> wrappers on required orchestrator subsystems
Type: grep-count
Acceptance: orchestrator sources contain zero matches for `Arc<Mutex<Option<` and zero matches for `Arc<RwLock<Option<` / `RwLock<Option<` around required constructor-owned subsystem fields.

#### VAL-A5-002: No Default backdoor on required orchestrator subsystems
Type: grep-count
Acceptance: workspace Rust sources contain zero `impl Default for` matches on the required subsystem set named by the refactor docs (`LlmBackend`-adjacent wrappers, `ExecutionController`, `ValidationOrchestrator`, `WorktreePool`, `PostToolHookManager`, `CommitStrategy`, `CostGuard`, `HookManager`).

#### VAL-A5-003: No with_* setter API remains on Orchestrator
Type: grep-count
Acceptance: `crates/swell-orchestrator/src/lib.rs` contains zero public `with_*` methods on `Orchestrator`.

#### VAL-A5-004: No disabled/null constructor backdoor exists on required subsystems introduced by this refactor surface
Type: grep-count
Acceptance: orchestrator/daemon-adjacent required subsystem types contain zero matches for `fn disabled`, `fn no_op`, or `fn null` used as production constructor substitutes.

#### VAL-A5-005: No OrchestratorConfig-with-Option hole replaces the old constructors
Type: grep-count
Acceptance: workspace Rust sources contain zero matches for an `OrchestratorConfig` carrying `Option<...>` fields for required orchestrator dependencies.

#### VAL-A5-006: Builder remains gated in source and in production artifact
Type: cargo-command
Acceptance: source-gating checks and the production builder-exclusion build both pass.

### Area A6 — Test redundancy and documentation handoff

#### VAL-A6-001: Redundant wiring test removed per redundancy audit
Type: grep-count
Acceptance: the compile-time-redundant `wiring_daemon_constructs_orchestrator_with_injected_llm` equivalent no longer exists in `crates/swell-integration-tests/tests/full_cycle_wiring.rs`.

#### VAL-A6-002: Obsolete daemon-LLM witness removed
Type: grep-count
Acceptance: `witness_daemon_does_not_depend_on_swell_llm` no longer exists.

#### VAL-A6-003: Runtime keeper wiring tests remain
Type: grep-count
Acceptance: the runtime-behavior keepers identified in `04_test_redundancy_audit.md` still exist in `crates/swell-integration-tests/tests/full_cycle_wiring.rs`.

#### VAL-A6-004: Wiring suite still passes after refactor 01
Type: cargo-command
Acceptance: `cargo test -p swell-integration-tests --test full_cycle_wiring` exits 0.

#### VAL-A6-005: Orchestrator AGENTS constructor policy updated
Type: grep-count
Acceptance: `crates/swell-orchestrator/AGENTS.md` contains a top-level `Constructor policy` section with the exact rule that production-required subsystems must be constructor parameters of `Orchestrator::new`.

#### VAL-A6-006: Integration-test AGENTS cross-reference updated
Type: grep-count
Acceptance: `crates/swell-integration-tests/AGENTS.md` cross-references the constructor refactor as the reason compile-time-redundant wiring tests were removed.

#### VAL-A6-007: Integration-test strategy note updated
Type: grep-count
Acceptance: `plan/audit-2026-04-16/07_integration_test_strategy.md` notes that the two constructor-presence checks moved from runtime wiring tests to compile-time invariants.

## Phase B — Refactor 02: Startup wiring manifest

### Area B1 — Trait and report surfaces

#### VAL-B1-001: WiringReport trait exists
Type: grep-count
Acceptance: a shared `WiringReport` trait exists in the planned shared crate surface.

#### VAL-B1-002: WiringState enum exists with Enabled/Degraded/Disabled
Type: grep-count
Acceptance: `WiringState` exists with explicit states for `Enabled`, `Degraded`, and `Disabled`.

#### VAL-B1-003: Orchestrator exposes a wiring manifest surface
Type: grep-count
Acceptance: `Orchestrator` exposes a `wiring_manifest`-style method returning report entries for held subsystems.

#### VAL-B1-004: Required subsystem reports are represented in the manifest
Type: grep-count
Acceptance: the manifest implementation names at least `LlmBackend`, `WorktreePool`, `ExecutionController`, `ValidationOrchestrator`, and `PostToolHookManager` (with disabled placeholders allowed if not yet landed).

### Area B2 — Startup output and strict mode

#### VAL-B2-001: Daemon prints startup wiring manifest exactly once
Type: grep-count
Acceptance: daemon startup path contains the manifest print call in a single startup location.

#### VAL-B2-002: Manifest lines expose explicit state tokens
Type: grep-count
Acceptance: startup manifest rendering includes explicit `enabled`, `DEGRADED`, or `DISABLED` state tokens.

#### VAL-B2-003: Manifest summary line exists
Type: grep-count
Acceptance: startup rendering includes a `[wiring-check]` summary line counting degraded/disabled subsystems.

#### VAL-B2-004: SWELL_STRICT blocks startup on degraded/disabled state
Type: cargo-command
Acceptance: the strict-mode validation command/test exits 0 and proves `SWELL_STRICT=1` refuses startup when any subsystem reports degraded/disabled.

#### VAL-B2-005: Wiring manifest integration test passes
Type: cargo-command
Acceptance: the manifest-specific `full_cycle_wiring` assertion or equivalent test exits 0 and proves required subsystem names appear.

#### VAL-B2-006: Daemon AGENTS wiring-manifest rule updated
Type: grep-count
Acceptance: `crates/swell-daemon/AGENTS.md` documents the startup manifest output/rule so future subsystems must update it.

## Phase B — Refactor 03: Newtype IDs

### Area B3 — Newtype definitions and migration

#### VAL-B3-001: SocketPath newtype exists
Type: grep-count
Acceptance: `SocketPath` exists in `swell-core::ids` (or the chosen shared ids module) as a transparent newtype.

#### VAL-B3-002: CommitSha newtype exists
Type: grep-count
Acceptance: `CommitSha` exists as a transparent newtype.

#### VAL-B3-003: BranchName newtype exists
Type: grep-count
Acceptance: `BranchName` exists as a transparent newtype with named access/validation methods.

#### VAL-B3-004: WorktreeId newtype exists
Type: grep-count
Acceptance: `WorktreeId` exists as a transparent newtype.

#### VAL-B3-005: AgentId newtype exists
Type: grep-count
Acceptance: `AgentId` exists as a transparent newtype.

#### VAL-B3-006: TaskId newtype exists
Type: grep-count
Acceptance: `TaskId` exists as a transparent newtype.

#### VAL-B3-007: No From<Uuid>/From<String> backdoor conversions on migrated IDs
Type: grep-count
Acceptance: the ids module contains zero `impl From<Uuid>` / `impl From<String>` for the migrated domain-ID wrappers.

#### VAL-B3-008: SocketPath migration compile check passes
Type: cargo-command
Acceptance: `cargo check --workspace` exits 0 after the SocketPath migration feature.

#### VAL-B3-009: CommitSha migration compile check passes
Type: cargo-command
Acceptance: `cargo check --workspace` exits 0 after the CommitSha migration feature.

#### VAL-B3-010: BranchName migration compile check passes
Type: cargo-command
Acceptance: `cargo check --workspace` exits 0 after the BranchName migration feature.

#### VAL-B3-011: WorktreeId migration compile check passes
Type: cargo-command
Acceptance: `cargo check --workspace` exits 0 after the WorktreeId migration feature.

#### VAL-B3-012: AgentId migration compile check passes
Type: cargo-command
Acceptance: `cargo check --workspace` exits 0 after the AgentId migration feature.

#### VAL-B3-013: TaskId migration compile check passes
Type: cargo-command
Acceptance: `cargo check --workspace` exits 0 after the TaskId migration feature.

#### VAL-B3-014: Raw public Uuid ID positions are eliminated from daemon/orchestrator surfaces
Type: grep-count
Acceptance: the filtered grep over `crates/swell-daemon/` and `crates/swell-orchestrator/` for `:\s*Uuid\b` reaches target 0 after excluding tests and legitimate ids-module definitions.

#### VAL-B3-015: ID-newtype rule documented
Type: grep-count
Acceptance: relevant crate guidance documents note that domain IDs are newtypes rather than raw `Uuid`/`String` API parameters.

## Phase B — Refactor 04: Daemon error enum

### Area B4 — Structured daemon error surface

#### VAL-B4-001: DaemonError enum exists
Type: grep-count
Acceptance: `DaemonError` exists in the daemon error surface with structured variants.

#### VAL-B4-002: DaemonError implements thiserror
Type: grep-count
Acceptance: `DaemonError` derives `thiserror::Error` or equivalent `std::error::Error` implementation via `thiserror`.

#### VAL-B4-003: DaemonErrorWire exists
Type: grep-count
Acceptance: a separate serializable wire/client error projection exists.

#### VAL-B4-004: Public daemon API returns DaemonError
Type: grep-count
Acceptance: public daemon functions no longer expose `anyhow::Result<_>` at the public boundary and instead use `Result<_, DaemonError>`.

#### VAL-B4-005: ValidationFailed variant exists
Type: grep-count
Acceptance: `DaemonError` contains a structured validation-failure class usable by negative-path tests.

#### VAL-B4-006: HookDenied variant exists
Type: grep-count
Acceptance: `DaemonError` contains a structured hook-denial class usable by negative-path tests.

#### VAL-B4-007: BudgetExceeded variant exists
Type: grep-count
Acceptance: `DaemonError` contains a structured budget-exceeded class usable by negative-path tests.

#### VAL-B4-008: No stringly error matching survives outside tests
Type: grep-count
Acceptance: workspace Rust sources outside tests contain zero matches for `err.*\.contains\(` and `error.*\.contains\(` on error-string matching paths.

#### VAL-B4-009: thiserror dependency is present
Type: grep-count
Acceptance: daemon crate dependencies include `thiserror`.

#### VAL-B4-010: Daemon error boundary tests pass
Type: cargo-command
Acceptance: daemon crate tests / targeted validation commands for the structured error surface exit 0.

## Phase C — Regression audit and cross-mission handoff

### Area C1 — Grep audit and handoff

#### VAL-C1-001: Refactor 01 metric reaches target
Type: grep-count
Acceptance: the `Option<...>` grep for load-bearing orchestrator subsystems is at target 0.

#### VAL-C1-002: Refactor 02 metric reaches target
Type: grep-count
Acceptance: the `impl WiringReport` metric reaches the target expected by the plan and is recorded in the Phase C artifact.

#### VAL-C1-003: Refactor 03 metric reaches target floor
Type: grep-count
Acceptance: the filtered raw `Uuid` grep across daemon/orchestrator surfaces reaches target 0 after excluding tests and legitimate ids-module definitions.

#### VAL-C1-004: Refactor 04 metric reaches target 0 outside tests
Type: grep-count
Acceptance: the stringly error-matching grep reaches target 0 outside tests.

#### VAL-C1-005: Cross-mission handoff edit landed
Type: grep-count
Acceptance: `plan/audit-2026-04-16/04_tier1_blockers.md` replaces the precondition banner with the documented one-line handoff note referencing the landed refactor-01 commit SHA.

#### VAL-C1-006: Final workspace validation is green
Type: cargo-command
Acceptance: final Phase C validation reruns `cargo check --workspace`, `cargo test --workspace`, `cargo test --workspace --doc`, and `cargo test -p swell-integration-tests --test full_cycle_wiring` successfully.
