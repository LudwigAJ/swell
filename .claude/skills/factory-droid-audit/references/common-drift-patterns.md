# Common Droid Drift Patterns

Observed failure modes from real sessions. Each entry: symptom → root cause → how to detect → how to fix.

## 1. "running 0 tests" misread as passing

**Symptom:** Droid reports "integration tests pass" citing output like `running 0 tests; 0 passed; 0 failed; 0 ignored; 0 filtered out`.

**Root cause:** The test filter matched nothing. Either (a) the test Droid thought it added doesn't exist yet, (b) all matching tests are `#[ignore]`'d, or (c) the test binary didn't compile with the required feature flag.

**Detection:**
```bash
# Enumerate tests instead of filtering — confirms what actually exists
cargo test -p <crate> --test <binary> -- --list
```
If the test Droid claims to have added isn't in the list, it wasn't added (or didn't compile).

**Fix:** Require Droid to quote the specific `test <name> ... ok` line from the output, not the summary.

## 2. Migration residuals

**Symptom:** Feature marked `completed` but acceptance-criteria grep returns non-zero.

**Root cause:** Droid commits a crate-by-crate migration but stops at the first compile-green point, leaving call sites in other crates untouched. The commit message says "migrate AgentId" but only `swell-orchestrator/src/agents.rs` was touched; call sites in `swell-daemon` and `swell-state` still use `Uuid`.

**Detection:**
```bash
# Per-crate residual count
for d in crates/*/; do
  count=$(rg -c "<pattern>" "$d" --type=rust 2>/dev/null | awk -F: '{s+=$2} END {print s+0}')
  echo "$d: $count"
done
```

**Fix:** Measurement greps must return 0 before the feature is marked complete. Do not accept "compiles" as a completion signal — the spec's acceptance criteria are the bar.

## 3. Unused-import leak after migration

**Symptom:** `cargo check --workspace` is green, but CI fails immediately with `error: unused imports`.

**Root cause:** Droid removed the last use of a type in a file but didn't remove the `use` statement. Default `cargo check` reports this as a warning; `RUSTFLAGS=-D warnings` (CI standard) promotes it to error.

**Detection:**
```bash
RUSTFLAGS="-D warnings" cargo check --workspace --all-targets
```

**Fix:** Either delete the imports or run `cargo fix --lib -p <crate> --allow-dirty` before committing.

## 4. Test-only feature leaked into production crate

**Symptom:** Droid adds `features = ["test-support"]` (or similar cfg-gated feature) to a production crate's Cargo.toml to make a test compile.

**Root cause:** Droid misreads a cfg-gate as a bug. The feature is intentionally gated to keep test-only symbols out of production binaries. Enabling it in a production crate's dependency graph pulls the test-only code into production.

**Detection:**
```bash
# For a project with an A-06-style gate
grep -rn "test-support" crates/*/Cargo.toml
# Or, more directly, re-run the CI production-build gate
cargo build --workspace --no-default-features --release --message-format=json | \
   grep -c '"name":"<ForbiddenSymbol>"'
```

**Fix:** Revert the Cargo.toml change. Move the test to a crate that legitimately has the feature enabled (typically an integration-tests crate), or construct the system-under-test via the production API path.

## 5. Amending commits cited in features.json

**Symptom:** `features.json` has a feature's `completedWorkerSessionId` pointing to a commit that no longer exists, or whose SHA has changed.

**Root cause:** Droid amended a commit after it was already cited, or rebased a branch that contained cited commits.

**Detection:**
```bash
# Extract cited SHAs from features.json and check each exists
jq -r '.features[].completedWorkerSessionId // empty' \
  ~/.factory/missions/<id>/features.json | \
  while read sha; do git cat-file -e "$sha" 2>/dev/null || echo "MISSING: $sha"; done
```

**Fix:** Add a follow-up commit; don't rewrite history. Add a rule to mission AGENTS.md: "Do not amend commits cited in `features.json` `fulfills`."

## 6. Silent spec divergence

**Symptom:** Feature compiles, tests pass, but the implementation doesn't match the spec. For example, the spec says "return `Box<dyn WiringReport>`" and Droid implements "return `Vec<WiringReport>`" because it's simpler.

**Root cause:** Droid optimizes for compile-green, not spec-match.

**Detection:** Read the feature's `expectedBehavior` array in features.json. For each bullet, confirm the implementation matches. A sampling of actual code against the spec catches this; the signature is usually the tell.

**Fix:** Flag divergence even if tests pass. The spec exists for downstream consumers; silent divergence creates rework later.

## 7. Scrutiny-validator false green

**Symptom:** `scrutiny/synthesis.json` says "all validators passed" but individual `reviews/*.md` files have blockers.

**Root cause:** The synthesis step summarizes across reviewers; if one reviewer raises a critical issue and others are green, the summary sometimes leans green.

**Detection:**
```bash
# Always read individual reviews, not just synthesis
ls ~/.factory/missions/<id>/.factory/validation/<milestone>/scrutiny/reviews/
# Or grep for blocker language
rg -n 'BLOCKER|CRITICAL|must fix|reject' \
   ~/.factory/missions/<id>/.factory/validation/<milestone>/
```

**Fix:** Require scrutiny synthesis to explicitly enumerate each reviewer's blockers, not just produce a pass/fail verdict.

## 8. Worker thrash on same feature

**Symptom:** A feature's `workerSessionIds` array has 3+ entries — multiple workers attempted the same feature.

**Root cause:** Workers kept failing (compile errors, test failures) and Droid handed the feature to the next worker. The 3rd or 4th worker eventually commits something, but the commit history shows fixup-on-fixup.

**Detection:**
```bash
jq -r '.features[] | select(.workerSessionIds | length > 2) |
       "\(.id): \(.workerSessionIds | length) workers"' \
  ~/.factory/missions/<id>/features.json
```

**Fix:** High worker count is a soft signal that the feature is under-specified or the spec conflicts with the codebase. Suggest the user break the feature into smaller features, or audit the final commit extra carefully for hacks.

## 9. "In progress" used as a shield

**Symptom:** A feature with lots of residuals is left as `status: in_progress` indefinitely, never forced to `completed` but also not being actively worked on.

**Root cause:** Droid's orchestrator sometimes advances to the next milestone while earlier features still show `in_progress`, on the theory that they'll be finished later.

**Detection:**
```bash
# Find in_progress features with no current worker
jq -r '.features[] | select(.status == "in_progress" and .currentWorkerSessionId == null) |
       .id' ~/.factory/missions/<id>/features.json
```

**Fix:** Flag to the user. These features either need a final worker run or need to be split (one commit done, one commit pending).
