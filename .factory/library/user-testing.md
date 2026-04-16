# User Testing

Testing surface, tools, and validation configuration for the audit-recovery mission.

---

## Validation Surface

### Primary: Wiring guardrail integration test
- Tool: `cargo test -p swell-integration-tests --test full_cycle_wiring`
- Purpose: prove daemon -> orchestrator -> execution -> validation -> worktree/git path is production-reachable
- This surface is the highest priority and must stay green once established

### Secondary: Crate-scoped cargo validation
- Tool: `cargo test -p <crate>`, `cargo check -p <crate>`, `cargo clippy -p <crate> -- -D warnings`
- Purpose: verify the local crate behavior for each audited feature without replacing the wiring guardrail surface

### Tertiary: CLI and daemon operator surfaces
- Tool: crate-scoped tests for `swell-cli`, `swell-daemon`, and audited integration surfaces
- Purpose: validate Tier 3 operator features after Tier 1 and Tier 2 gates are green

## Validation Concurrency

### Wiring guardrail surface
- Max concurrent validators: 1
- Rationale: this mission is specifically sensitive to flaky validation and shared runtime wiring; serialize this suite to avoid ambiguous failures.

### Crate-scoped cargo validation
- Max concurrent validators: 3
- Rationale: 10 CPU cores on this machine, but keep headroom for heavy orchestrator/integration compilation and avoid false negatives from resource contention.

### CLI/daemon surfaces
- Max concurrent validators: 2
- Rationale: local filesystem/socket coordination and orchestrator-heavy runtime tests can interfere if overscheduled.

## Mission-Specific Rules

- Never treat crate-local green as sufficient for Tier 1 wiring work.
- After every Tier 1 feature, re-run the wiring guardrail surface relevant to the changed assertion(s).
- After every Tier 2 feature, confirm no Tier 1 guardrail regression.
- Tier 3 work must preserve already-green Tier 1 and Tier 2 gates.
- Workers and validators must update the mission's machine-readable progress artifacts when reporting checkbacks.
