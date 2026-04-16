# Mission-Specific Worker Rules

These rules are additive to the repository `AGENTS.md` and exist only because this mission is optimized for a flaky, context-limited worker pool.

## 1. Feature execution discipline

- Do not read the full source-plan folder during implementation unless the feature spec explicitly requires it. The feature spec is the source of truth for the worker.
- Only edit the file paths named in the feature’s `worker_spec.files` list. If a truly necessary extra file emerges, return to the orchestrator instead of widening scope silently.
- One feature must produce one green, revertable commit.

## 2. Validator discipline

- Validators do not fix unrelated repo issues.
- Validators do not run opportunistic formatting or cleanup outside the feature’s named scope.
- Validators stop at the first failing step and return the exact `fail_reason` from the feature spec.

## 3. Phase discipline

- Phase A must be fully green before any Phase B feature starts.
- Phase B must be fully green before Phase C starts.
- No edits under `plan/audit-2026-04-16/` are allowed until Phase C, except none. Phase C may edit only `plan/audit-2026-04-16/04_tier1_blockers.md` for the documented handoff line.

## 4. Anti-pattern discipline for Refactor 01

When touching the orchestrator constructor surface, the following are forbidden and must trigger a return to orchestrator if they appear necessary:

- `Arc<Mutex<Option<...>>>` or equivalent wrapped-option holes for required dependencies
- `Default` implementations that create silent required-subsystem placeholders
- public `with_*` setter APIs on `Orchestrator`
- `disabled()` / `null()` / `no_op()` constructor substitutes for required dependencies
- `OrchestratorConfig` with `Option` fields for required dependencies
- exposing `OrchestratorBuilder` to production code

## 5. Handoff note requirement

After completing a feature, write exactly one handoff note file under:

`.factory/missions/2fbb8327-d5b3-4c25-9643-c08a4140b7c7/handoffs/`

The note must include:
- what landed,
- commit SHA,
- which validation IDs are now expected to pass.
