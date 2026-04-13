# Parity Source of Truth

## What makes PARITY.md canonical

`references/claw-code/PARITY.md` at the repo root is the single canonical source for ClawCode's behavioral parity status. This is not a convention — it is enforced by the diff workflow itself.

The top of PARITY.md states it explicitly:

> **Canonical document**: this top-level `PARITY.md` is the file consumed by `rust/scripts/run_mock_parity_diff.py`.

This means every behavioral claim in the Rust implementation's mock parity harness is validated against the text of PARITY.md, not against a config file or a hardcoded comment.

## How the diff workflow uses PARITY.md

The diff workflow is implemented in `rust/scripts/run_mock_parity_diff.py`. The script operates in two phases: a reference check phase and a harness execution phase.

### Phase 1 — Reference validation before any test runs

The script loads two inputs:

- `rust/mock_parity_scenarios.json` — the scenario manifest, which maps each test scenario to one or more `parity_refs` strings.
- The full text of `PARITY.md` — loaded as a single string via `load_parity_text()`.

Before running any tests, `ensure_refs_exist()` iterates every entry in the scenario manifest and checks that each `parity_refs` string appears verbatim in the PARITY.md text. If any reference is missing, the script exits with code 1 and prints:

```
Missing PARITY.md references:
  - <scenario_name>: <missing_ref>
```

This means the PARITY.md text acts as a behavioral allowlist: a scenario cannot claim to validate a behavior that PARITY.md does not mention. If a developer removes or renames a claim from PARITY.md without updating the scenario manifest, the diff script fails immediately — before the harness even runs.

The scenario manifest is the join table:

```json
{
  "name": "write_file_denied",
  "category": "permissions",
  "parity_refs": [
    "Mock parity harness \u2014 milestone 1",
    "Permission enforcement across tool paths"
  ]
}
```

Both of those `parity_refs` strings must be present in PARITY.md for this scenario to pass the reference check.

### Phase 2 — Harness execution and report mapping

After reference validation succeeds, the script runs the mock parity harness:

```python
subprocess.run(
    [
        "cargo",
        "test",
        "-p",
        "rusty-claude-cli",
        "--test",
        "mock_parity_harness",
        "--",
        "--nocapture",
    ],
    cwd=rust_root,
    check=True,
    env={**env, "MOCK_PARITY_REPORT_PATH": str(report_path)},
)
```

The harness test produces a JSON report at the path specified by `MOCK_PARITY_REPORT_PATH`. The report contains one entry per scenario: `iterations`, `request_count`, `tool_uses`, `tool_errors`, and `final_message`.

The diff script then matches scenario names from the manifest against entries in the report. For each matched scenario it prints a `PASS` status; for any manifest entry without a corresponding report entry it prints `MISSING`. This distinction matters: `MISSING` means the harness did not produce a report for that scenario, which typically means the test panicked or was filtered out. `PASS` means the harness exercised the scenario end-to-end and produced structured output.

### Phase 3 — Coverage map

After reporting individual scenario results, the script emits a coverage map that inverts the scenario→ref relationship:

```python
coverage = defaultdict(list)
for entry in manifest:
    for ref in entry["parity_refs"]:
        coverage[ref].append(entry["name"])
```

For every distinct `parity_refs` string, the map lists all scenarios that claim to validate it. This is the mechanism that lets a reviewer answer: "which scenarios validate `Permission enforcement across tool paths`?" The answer is produced by the script, not inferred by hand.

## The two-file contract

The workflow depends on two files staying in sync:

1. **`PARITY.md`** — the behavioral narrative. Claims what the Rust implementation should do and which scenarios validate each claim.
2. **`rust/mock_parity_scenarios.json`** — the scenario manifest. Names scenarios, categorizes them, and lists the `parity_refs` strings that must exist in PARITY.md.

The script enforces that every entry in the manifest has at least one reference in the narrative, and that every reference in the manifest is actually present in the narrative. This creates a bidirectional contract: no scenario without a claim, no claim without a scenario.

The `parity_refs` strings use Unicode dash characters (`\u2013`, `\u2014`) as punctuation inside the strings, so the manifest must use those same characters verbatim. This is a known fragility in the current implementation — copying and reformatting the strings without preserving the Unicode characters breaks the reference check silently.

## Evidence from the repo

- `references/claw-code/PARITY.md` — states canonical status in the Summary section.
- `references/claw-code/rust/scripts/run_mock_parity_diff.py` — the enforcement script; `load_parity_text()` reads PARITY.md, `ensure_refs_exist()` validates references.
- `references/claw-code/rust/mock_parity_scenarios.json` — the scenario manifest; `parity_refs` arrays are the join key between scenarios and PARITY.md sections.
- `references/claw-code/rust/MOCK_PARITY_HARNESS.md` — documents the run command (`python3 scripts/run_mock_parity_diff.py`) and the scenario-to-PARITY mapping responsibility.

## Builder lessons

**The enforce-by-default pattern is worth replicating.** The diff script does not merely recommend that PARITY.md stay current — it makes staleness a hard failure. The same pattern applies to any feature contract: if the doc does not say it, the test does not run. This keeps documentation from drifting from implementation in the normal course of development.

**Unicode normalization matters in string-join contracts.** The `parity_refs` strings use Unicode em-dashes and en-dashes that are visually identical to ASCII hyphens. A string comparison that does not account for this will silently fail to match, producing a spurious missing-reference error. When writing tools that join structured data against narrative text, preserve Unicode characters exactly or normalize upfront.

**Coverage maps inverted from the manifest are more actionable than hand-maintained tables.** The script builds the coverage map from the same data it uses for execution, so it cannot go stale. Any time a new scenario is added with a reference, the map updates automatically on the next run. Hand-maintained tables require a separate discipline that tends to break down under pressure.

**The report path is environment-variable driven, not path-based.** The harness writes to `MOCK_PARITY_REPORT_PATH` which the script sets into the subprocess environment. This means the same binary can produce reports to different locations depending on context (CI temp dir vs. developer-specified path). This is a cleaner pattern than embedding paths in test code or using a fixed conventional location.
