# Writing Mission Guardrails

When you find a recurring drift pattern, add a guardrail to the mission's `AGENTS.md`. This file is read by every Droid worker at the start of a session.

## Where guardrails go

| Scope | File | Lifespan |
|-------|------|----------|
| Mission-specific (this mission only) | `~/.factory/missions/<id>/AGENTS.md` | Dies with the mission |
| Crate-local, cross-mission | `crates/<crate>/AGENTS.md` | Permanent; applies to all future work in that crate |
| Repo-wide | `AGENTS.md` at repo root | Permanent; applies to every worker |

**Default to mission-scoped.** Mission AGENTS.md files die with the mission and don't pollute future unrelated work. Only elevate to crate or repo scope when the rule is fundamental and permanent (e.g. "no `unsafe` without a safety comment" is repo-wide; "this migration must use `#[serde(transparent)]`" is mission-scoped).

## Guardrail template

Every guardrail has four parts:

```markdown
## N. [Short, imperative title]

**[One-sentence rule, in bold.]**

**Why:** [One paragraph — the past incident or the load-bearing reason. Future workers need to know *why* so they can judge edge cases, not just follow rules blindly.]

**Load-bearing facts:**
- `path/to/file.rs:LINE` — what's there and why it matters
- CI job name in `.github/workflows/*.yml:LINE` — what gate will fail if you violate this
- Spec reference `plan/<area>/<doc>.md` — what authority this rule derives from

**Legitimate alternatives (in order of preference):**
1. [Most-preferred path — usually "move the test to the right crate" or "use the production API"]
2. [Second choice with tradeoffs noted]
3. [Escape hatch with explicit approval required]

**If you still believe you need to violate this rule, stop and surface the question to the user.**
```

## Example (from Phase B mission)

```markdown
## 6. STOP — do not enable test-support on swell-daemon

**`swell-daemon` must not pull in `swell-orchestrator/test-support`.**

**Why:** This is enforced by CI at `.github/workflows/ci.yml:17-35` (`build-no-default-features` job). Adding `test-support` to the daemon's `swell-orchestrator` dependency pulls `OrchestratorBuilder` and `Orchestrator::new_for_test()` into the production build graph. That defeats the entire Phase A refactor.

**Load-bearing facts:**
- `Orchestrator::new_for_test()` at `crates/swell-orchestrator/src/lib.rs:452` is gated `#[cfg(any(test, feature = "test-support"))]`
- `pub use builder::OrchestratorBuilder;` at `crates/swell-orchestrator/src/lib.rs:82-83` is gated the same way
- `crates/swell-integration-tests/Cargo.toml:31` declares `swell-orchestrator` with `features = ["test-support"]` — tests in *that* crate have access

**Legitimate alternatives:**
1. **Move the test to `crates/swell-integration-tests/tests/`** — already has test-support, is the canonical home for wiring guardrails.
2. **Construct via the production path** — `Orchestrator::new(Arc::new(swell_llm::MockLlm::new("test")))`. No feature flag needed.
3. **Pass `--features swell-orchestrator/test-support` on the command line** — no Cargo.toml change.

**If you still believe you need to edit a Cargo.toml features table, stop and surface the question to the user.**
```

## Anti-patterns in your own guardrails

- **Don't write "don't do X" without explaining why.** Workers will find exceptions and violate the rule. Give them the reason so they can judge.
- **Don't cite a file without a line number.** Workers will struggle to find the reference and may skip verification.
- **Don't leave escape hatches implicit.** "Ask the user" must be an explicit final step, or workers will invent their own escape.
- **Don't stack guardrails past ~10 per mission.** Too many rules blur into noise. Consolidate when possible; graduate permanent ones to the crate-level AGENTS.md.

## When to retire a guardrail

After the mission completes, re-evaluate:

1. Was the guardrail triggered? If no, it was unnecessary — don't carry it forward.
2. If triggered, is it permanent? Move to crate or repo AGENTS.md.
3. If one-off, archive with the mission (the file dies naturally when the mission is cleaned up).

## Cross-mission signals

If the same guardrail keeps reappearing across missions (e.g. "don't disable clippy lints"), that's a signal to promote it to the repo-root `AGENTS.md` with a permanent rationale.
