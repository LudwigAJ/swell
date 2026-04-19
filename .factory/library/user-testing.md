# User Testing

Testing surface, tools, and validation configuration for the Structural Refactors Phase B+C mission.

---

## Validation Surface

### Primary: Wiring guardrail integration test
- Tool: `cargo test -p swell-integration-tests --test full_cycle_wiring`
- Purpose: prove every subsystem wired into `Orchestrator` is production-reachable from `Daemon::run`
- This surface is the highest priority — must stay green once established

### Secondary: Crate-scoped cargo validation per feature
- Tool: `cargo check -p <crate>`, `cargo test -p <crate>`, `cargo clippy -p <crate> -- -D warnings`
- Purpose: verify local crate behavior for each refactor without replacing the wiring guardrail surface

### Tertiary: Workspace-wide validation (final gate)
- Tool: `cargo test --workspace`, `cargo check --workspace`, `cargo clippy --workspace -- -D warnings`
- Purpose: final regression check after all refactors land

---

## Validation Concurrency

### Wiring guardrail surface
- Max concurrent validators: 1
- Rationale: mission is specifically sensitive to flaky validation and shared runtime wiring; serialize this suite to avoid ambiguous failures.

### Crate-scoped cargo validation
- Max concurrent validators: 3
- Rationale: heavy orchestrator/integration compilation can contend for resources; keep headroom.

### Workspace-wide validation
- Max concurrent validators: 1
- Rationale: full workspace test/build/link is resource-intensive; run serially as final gate.

---

## Mission-Specific Rules

- **Phase A binding rule must be preserved.** No `Option<_>` around production-required subsystems. Workers must run the Phase A binding rule grep before marking a refactor complete.
- **Refactors can land independently.** Each refactor (02, 03, 04) is independently reviewable and mergeable. If two refactors have circular compilation dependencies, that is a failure condition.
- **Never treat crate-local green as sufficient.** For refactors that touch multiple crates, run `cargo check --workspace` before claiming completion.
- **Measurement greps are the audit trail.** Before/after grep numbers must appear in every Phase C checkback.
- After every feature: re-run the wiring guardrail surface relevant to the changed assertion(s).
- Tier 3 work must preserve already-green Tier 1 and Tier 2 gates (not applicable here — this is a refactor mission).

---

## Testing Approach for This Mission

This mission is purely a refactor mission with no new feature surface. The testing strategy is:

1. **Compile-time checks** (primary): `cargo check` gates catch type mismatches, missing implementations, and wrong return types.
2. **Workspace-wide compilation**: `cargo check --workspace` ensures no cross-crate breakage.
3. **Test suite**: `cargo test --workspace` ensures no runtime behavior regression.
4. **Wiring guardrail**: `cargo test -p swell-integration-tests --test full_cycle_wiring` proves production reachability.
5. **Measurement greps**: The 4 greps from `00_README.md` provide quantitative audit trail.

**No manual user-facing testing required** — this mission touches no user-facing surfaces (no CLI changes, no API changes, no UI). All assertions are code-level or integration-level.
