---
name: factory-droid-audit
description: Audits Factory Droid mission progress — verifies worker swarm output against mission spec, cross-references claimed completions with git history, runs acceptance-criteria greps, and catches common drift patterns (unused imports, "0 tests passed", test-only features leaking into production, partial migrations claimed as complete). Use when the user asks about Droid progress, whether the swarm "did its job", or wants to review a mission milestone before advancing.
---

# Factory Droid Mission Audit

## When to use

- User reports Droid completed features and wants verification.
- Before advancing a mission to the next milestone.
- When Droid reports "all tests pass" and you need to confirm it's true.
- When the user wants to add guardrails to prevent a recurring failure pattern.

## Mental model

Factory Droid is a swarm of AI workers coordinated by an orchestrator that breaks a mission into ordered features. Workers claim features, commit code, and mark features `completed` in `features.json`. **Droid self-reports are not authoritative — git history and spec-driven verification are.** Your job is to confirm that what Droid *says* happened matches what *actually* happened, and that both match the mission spec.

## Mission file layout

Under `~/.factory/missions/<mission-id>/`:

| File | Purpose | Audit value |
|------|---------|-------------|
| `mission.md` | Human-readable mission overview, milestones, acceptance criteria | Source of truth for scope |
| `features.json` | Ordered feature breakdown with status, preconditions, verificationSteps, fulfills | Primary audit target — cross-check against git |
| `validation-contract.md` | VAL-* assertion IDs that features fulfill | Maps features to measurable checks |
| `validation-state.json` | Current state of each VAL-* assertion | Cross-check against actual repo state |
| `AGENTS.md` | Mission-specific rules for workers | Where you add guardrails |
| `progress_log.jsonl` | Orchestrator event log | Timeline reconstruction |
| `worker-transcripts.jsonl` | Per-worker transcripts | Debugging individual worker mistakes |
| `handoffs/` | Inter-milestone handoff artifacts | Synthesis checkpoints |
| `.factory/validation/<milestone>/` | Scrutiny + user-testing validator output | Milestone closure evidence |

Supporting files in the repo:
- `.factory/services.yaml` — the commands scrutiny runs (test, check, lint, fmt, custom gates)
- `.factory/library/` — shared patterns workers reference
- `crates/*/AGENTS.md` — crate-local rules workers must follow

## Audit workflow

Run this sequence. Each step can falsify "Droid did its job":

1. **Load mission context** — Read `mission.md` for scope, `features.json` for the feature list, and the mission `AGENTS.md` for rules. Note which features are `status: completed` vs `in_progress` vs `pending`.
2. **Cross-reference completions with git** — For each `completed` feature, find the matching commit(s). `git log --oneline -30` is usually enough. If no commit exists, the completion is fiction.
3. **Run the feature's own `verificationSteps`** — features.json lists them. These are the acceptance criteria Droid is meant to hit. Run them verbatim.
4. **Run the mission's measurement greps** — Most structural missions have greps in `mission.md` (e.g. `rg -c ':\s*Uuid\b'`). Compare current numbers against the spec's target.
5. **Run CI-equivalent locally** — Read `.github/workflows/ci.yml` for `RUSTFLAGS`, feature flags, and custom gates. Local `cargo check` without `RUSTFLAGS=-D warnings` hides real CI failures.
6. **Check A-gates / binding rules** — Every mission inherits earlier-phase invariants (e.g. Phase A antipattern gate). Confirm none regressed.
7. **Report** — Separate "intact", "in progress with evidence", and "broken/misreported". Cite file paths and line numbers.

See `references/audit-workflow.md` for the detailed command sequence.

## Common drift patterns

Load `references/common-drift-patterns.md` for the full catalogue. The high-frequency ones:

- **"running 0 tests" misread as passing** — An empty filter match is NOT a pass. Require the named test in the output.
- **Migration residuals** — Droid commits a partial migration (e.g. `fix(agent_id)` touches 2 files, leaves 12 call sites) and marks the feature complete.
- **Unused-import leak** — `cargo check` green but CI's `RUSTFLAGS=-D warnings` rejects. Always check locally with `-D warnings` before trusting Droid.
- **Test-only feature leaked into production** — Droid adds `features = ["test-support"]` to a production-crate dep to unblock a test. Breaks the production-build gate.
- **Commit amending** — Droid amends a commit already cited in `features.json` `fulfills`. Breaks audit traceability.
- **Silent spec divergence** — Droid implements a workaround that compiles, but doesn't match the mission's acceptance criteria.

## Common guardrail patterns

When you find a recurring failure, add it to the mission's `AGENTS.md`, not the repo-level AGENTS.md — mission rules die with the mission and don't accumulate. Pattern:

1. Name the anti-pattern observed (e.g. "STOP — do not enable test-support on swell-daemon").
2. Cite the load-bearing facts with file paths and line numbers.
3. List the legitimate alternatives in order of preference.
4. End with: "If you still believe you need to do X, stop and surface the question to the user."

See `references/guardrails.md` for templates.

## Reporting format

Use three sections, in this order:

```
### Completed items (verified against git)
| Item | Commit | Status |

### 🟢 What's working
- Gate X intact
- Acceptance criterion Y met

### 🟡 In progress / partial
- Feature Z: X% done against measurement grep
- Residuals listed with line counts

### 🔴 Broken / misreported
- CI will fail because ...
- Acceptance criterion Y claimed but grep shows ...
```

Always cite file:line. Always include the exact command you ran. Don't trust Droid's self-report numbers — re-run the greps.

## What NOT to do

- Do not spawn subagents to audit. The audit is cheap — `git log`, a few `rg`s, one `cargo check`. Delegation burns context and produces a less accurate report than doing it inline.
- Do not edit features.json, validation-state.json, or any `.factory/missions/<id>/*.json` directly. Those are Droid's state machine; editing them causes Droid to desynchronize. Exception: explicit user instruction.
- Do not tell the user "looks good" without running verification. Droid's cheerful tone ("All tests pass!") is usually wrong in at least one measurable way.
- Do not add guardrails to `CLAUDE.md` or repo-root `AGENTS.md` for mission-specific issues. Those live in the mission's `AGENTS.md`.
