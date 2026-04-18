# swell-integration-tests AGENTS.md

## Purpose

This crate is **not** where you prove a feature works in isolation. That is the
job of each feature's own unit tests inside its home crate.

This crate is where we prove that features are **connected** to the daemon —
that the primary runtime entry point (`swell-daemon::Daemon`) can reach them
through production wiring, not through test-only builders and not with mocks
substituted for the components under test.

Swell's dominant failure mode is orphan features: built, unit-tested, green
in CI, and never wired in. Unit tests cannot catch orphans because they load
the module they test. Integration tests here cross the daemon boundary and
refuse to go green until the wires exist.

> Cross-ref: the `Orchestrator` constructor policy (required-subsystem rule,
> antipattern catalogue, `Arc::new_cyclic` signature deviation) lives in
> `crates/swell-orchestrator/AGENTS.md`. Wiring tests here consume
> `OrchestratorBuilder` under `--features swell-orchestrator/test-support`,
> which is cfg-gated out of production by the `build-no-default-features` CI
> job.

## Test binaries

### `prompt_integration_tests.rs`

Narrower scope: proves `ValidationOrchestrator::validate_task_completion`
behaves correctly against scripted scenarios. Does **not** assert daemon
wiring. Keep as-is.

### `full_cycle_wiring.rs`

The wiring guardrail suite. Two categories of tests:

- `wiring_*` — assertions of what Tier 1/2 wiring must look like. Currently
  `#[ignore]`-gated with messages pointing at
  `plan/audit-2026-04-16/04_tier1_blockers.md` and `05_tier2_reliability.md`.
  When a blocker ships, the matching test becomes an acceptance check.
- `witness_*` — assertions of today's **broken** state. They exist to force
  the swarm to notice when they fix a wire and should un-ignore the
  corresponding invariant. A red witness = "the fix landed, complete the
  follow-through."

Rules (same as root `AGENTS.md` — duplicated here for emphasis):

1. Do not delete tests.
2. Do not rewrite wiring tests as pure-mock unit tests.
3. If an API change breaks compilation, update the test. The test is the
   contract.
4. When you complete a Tier 1/2 blocker:
   - Remove the matching `#[ignore]`.
   - Delete the matching `witness_*` (if any).
   - Both in the same commit.
5. New wiring invariants = new `wiring_<subject>_<invariant>` test. Name the
   invariant so a failure message locates the broken wire.

## Why not `tests/` at the repo root?

Workspace-level `tests/` do not exist in this repo (only `crates/*/tests/`).
Placing the guardrail suite inside a workspace crate keeps it in the default
`cargo test --workspace` run and forces it through the same build path as
production code. A standalone binary at the root would be easier to miss and
easier to silently drop from CI.

## Extending this crate

Any new subsystem that the daemon must invoke should get at least one
`wiring_*` test here before it is considered "integrated." Start with an
`#[ignore]`'d test and a companion `witness_*` that confirms the wire is
missing today. Land both in the same PR as the feature scaffolding, so the
wiring work has a clear acceptance target.

## Running

```bash
# Wiring suite only
cargo test -p swell-integration-tests --test full_cycle_wiring

# All integration tests
cargo test -p swell-integration-tests

# Include the currently-ignored wiring tests (they will fail — that is the point)
cargo test -p swell-integration-tests --test full_cycle_wiring -- --ignored
```

`--ignored` is useful locally to see exactly which Tier 1/2 invariants still
need work. Don't run `--ignored` in CI — that would turn "blocker not done
yet" into "build red" and create pressure to silently soften the tests.
