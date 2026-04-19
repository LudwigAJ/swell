# Audit Workflow — Detailed Commands

The exact command sequence for a milestone audit. Adapt paths to the current mission.

## Step 1 — Locate the mission

```bash
ls ~/.factory/missions/
```

Pick the mission UUID from user context. Read in this order:

1. `~/.factory/missions/<id>/mission.md` — scope, milestones, acceptance criteria
2. `~/.factory/missions/<id>/features.json` — feature list with status
3. `~/.factory/missions/<id>/AGENTS.md` — existing guardrails
4. `~/.factory/missions/<id>/validation-contract.md` — VAL-* assertion list

## Step 2 — Map completions to commits

```bash
git log --oneline -30
```

For each feature in `features.json` with `status: completed`, find the matching commit. The feature's `description` usually names the artifact (e.g. "Define 9 newtype wrappers in ids.rs" → look for `feat(core): define 9 newtype wrappers`).

If a `completed` feature has no commit:
- Check `workerSessionIds` — may be scrutiny/validator features that don't produce commits, only synthesis files under `.factory/validation/<milestone>/`.
- If it's an implementation feature with no commit, flag as misreported.

## Step 3 — Run feature verificationSteps

Each feature in features.json has a `verificationSteps` array. Run them verbatim.

Common patterns:
- `cargo check -p <crate>` — per-crate compile
- `cargo test -p <crate> <filter>` — targeted test
- `rg <pattern>` — presence/absence check against acceptance criteria

If a verification step is missing or vague, fall back to `expectedBehavior` (also in features.json).

## Step 4 — Run mission measurement greps

Most structural missions have before/after greps in `mission.md` under a "Phase C" or "Regression Audit" section. Run each, record the number, compare to the spec's target.

Example patterns to look for in `mission.md`:
- `rg -c 'Option<.*Subsystem>' crates/ --type=rust`
- `rg -c ':\s*Uuid\b' crates/ --type=rust | rg -v 'ids\.rs|uuid::Uuid'`
- `rg -c 'err.*\.contains\(' crates/ --type=rust -g '!**/tests/**'`

## Step 5 — Run CI-equivalent locally

Read `.github/workflows/ci.yml` for the exact commands and environment. Key items:

- `env.RUSTFLAGS` — often `-D warnings`, which promotes unused imports to errors. Locally reproduce: `RUSTFLAGS="-D warnings" cargo check --workspace`.
- Feature flags — CI often runs `cargo test --features some-crate/test-support`. Without the flag, tests may silently not compile.
- Custom gates — search ci.yml for `grep`, `rg`, or shell scripts that check for antipatterns.

Run the specific CI jobs that would gate this mission's changes:

```bash
# Default-feature production build (catches leaked test-only symbols)
cargo build --workspace --no-default-features --release

# Warnings-as-errors compile
RUSTFLAGS="-D warnings" cargo check --workspace --all-targets

# Fmt + clippy
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings

# Test suite with the same features as CI
cargo test --workspace --features <crate>/test-support
```

## Step 6 — Check inherited binding rules

Every later-phase mission inherits earlier-phase invariants. Verify they haven't regressed. Typical checks for a Rust mission with Phase A complete:

```bash
# No Option-wrapped required subsystems (Phase A antipattern)
rg 'Option<.*(LlmBackend|WorktreePool|ValidationOrchestrator|PostToolHookManager|ExecutionController|CheckpointManager|FileLockManager|McpConfigManager)>' \
   crates/swell-orchestrator/src/lib.rs

# No test-support leaking into production crates
grep -n "test-support" crates/swell-daemon/Cargo.toml

# No forbidden escape hatches (unsafe, raw pointers) in constructor paths
rg '\*const |unsafe impl|Arc::from_raw|std::ptr::null' crates/swell-orchestrator/src/
```

If any of these return hits, the mission has regressed earlier-phase gates.

## Step 7 — Write the report

Structure per `SKILL.md`:

1. **Verified completions table** (item → commit)
2. **🟢 What's working** (gates intact, criteria met)
3. **🟡 In progress with evidence** (migration X% done, N residuals at file:line)
4. **🔴 Broken / misreported** (CI will fail because, acceptance criterion claimed but unmet)

Always include:
- File paths + line numbers for every finding
- The exact command that produced the finding (so the user can re-run)
- A short "what I'd tell Droid next" list with concrete next commits

## Anti-patterns in your own audit

- **Don't trust `cargo check` alone.** It's green under default flags but CI uses `-D warnings`. Always add `RUSTFLAGS="-D warnings"` for the final pass.
- **Don't audit with `cargo test --workspace` alone.** Without the right `--features` flag, test binaries may not even compile and you'll get "0 failures" on an empty run.
- **Don't trust `.factory/validation/<milestone>/scrutiny/synthesis.json`** without skimming the individual `reviews/*.md` files. Synthesis occasionally smooths over issues the reviewers flagged.
- **Don't stop at the first green signal.** Droid's failure pattern is "mostly green with one critical leak." Keep running checks until you've either found a leak or exhausted the spec's verification list.
