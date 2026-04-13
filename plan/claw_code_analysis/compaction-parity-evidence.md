# Compaction Parity Evidence — Roadmap, Parity, and Harness Citations

## What This Document Covers

This document shows how ClawCode's session compaction behavior is evidenced across the roadmap, parity documentation, and the mock parity harness — tracing the chain from stated intent through implementation to automated validation. It covers what the named scenario `auto_compact_triggered` proves, where the relevant citations live in the repo, and how they connect to each other. For the compaction mechanism itself (trigger logic, tail preservation, tool-pair boundaries, synthetic continuation messages, summary shape, and merge behavior), see `analysis/compaction-trigger-tail.md` and `analysis/compaction-resume-packet.md`.

**Evidence sources:**
- `references/claw-code/ROADMAP.md` — roadmap intent and dogfooding notes
- `references/claw-code/PARITY.md` — top-level parity status and open gaps
- `references/claw-code/rust/PARITY.md` — Rust-workspace parity status
- `references/claw-code/rust/mock_parity_scenarios.json` — scenario manifest entry
- `references/claw-code/rust/crates/rusty-claude-cli/tests/mock_parity_harness.rs` — harness implementation

---

## The Named Scenario: `auto_compact_triggered`

The single compaction-specific scenario in the mock parity harness is `auto_compact_triggered`.

### Scenario manifest entry

From `references/claw-code/rust/mock_parity_scenarios.json`:

```json
{
  "name": "auto_compact_triggered",
  "category": "session-compaction",
  "description": "Verifies auto-compact fires when cumulative input tokens exceed the configured threshold.",
  "parity_refs": [
    "Session compaction behavior matching",
    "auto_compaction threshold from env"
  ]
}
```

The `category` field places this in the `session-compaction` cluster. The `parity_refs` map to two distinct parity claims:

1. **"Session compaction behavior matching"** — the runtime compaction behavior matches the documented/expected shape
2. **"auto_compaction threshold from env"** — the threshold is configurable via environment variable

### What the harness validates

`assert_auto_compact_triggered()` in `mock_parity_harness.rs` verifies:

1. The response contains the message `"auto compact parity complete."`
2. The `auto_compaction` key is present in the JSON output (format parity)
3. The `input_tokens` field reflects the large mock token counts seeded by the fixture

The fixture (`prepare_auto_compact_fixture`) seeds a session with 6 pre-populated messages so that the cumulative input token count crosses the auto-compaction threshold on the next turn.

```rust
fn assert_auto_compact_triggered(_: &HarnessWorkspace, run: &ScenarioRun) {
    assert_eq!(run.response["iterations"], Value::from(1));
    assert_eq!(run.response["tool_uses"], Value::Array(Vec::new()));
    assert!(
        run.response["message"]
            .as_str()
            .expect("message text")
            .contains("auto compact parity complete."),
        "expected auto compact message in response"
    );
    assert!(
        run.response
            .as_object()
            .expect("response object")
            .contains_key("auto_compaction"),
        "auto_compaction key must be present in JSON output"
    );
    // Verify input_tokens field reflects the large mock token counts
    let input_tokens = run.response["usage"]["input_tokens"]
```

The assertion checks that the turn completes without tool uses (because compaction summarized the prior context), that the response message acknowledges the compaction event, and that the `auto_compaction` field appears in the structured JSON output — proving the runtime surfaced the compaction event to callers.

### What it does not cover

The harness scenario covers **format parity** and **trigger behavior** — that compaction fires and that the runtime reports it in the turn output. It does not cover the internal quality of the summary or the preservation boundary logic; those are validated by unit tests in `compact.rs` (e.g., `compaction_does_not_split_tool_use_tool_result_pair`, `keeps_previous_compacted_context_when_compacting_again`).

---

## Roadmap Evidence

### ROADMAP.md — Item 38: Dead-session opacity

The most compaction-specific roadmap entry is item 38 in `references/claw-code/ROADMAP.md`:

> **Dead-session opacity: bot cannot self-detect compaction vs broken tool surface** — dogfooded 2026-04-09. Jobdori session spent ~15h declaring itself "dead" in-channel while tools were actually returning correct results within each turn. Root cause: context compaction causes tool outputs to be summarised away between turns, making the bot interpret absence-of-remembered-output as tool failure.

**Status: Done (verified 2026-04-11).** The fix added a post-compaction session-health probe through `glob_search` that fails fast with a targeted recovery error if the tool surface is broken, and skips the probe for a freshly compacted empty session.

This item is significant because it ties compaction directly to a real failure mode observable in dogfooding. The fix (`ConversationRuntime::run_turn()` now runs a post-compaction health probe) is a behavioral addition that the parity harness does not yet cover — the `auto_compact_triggered` scenario validates the trigger and output shape, but not the health probe path.

### ROADMAP.md — Runtime Behavioral Gaps

The roadmap also lists compaction under **Runtime Behavioral Gaps**:

```
## Runtime Behavioral Gaps
- [ ] Session compaction behavior matching
- [ ] Token counting / cost tracking accuracy
```

"Session compaction behavior matching" is marked as **still open** in the top-level `PARITY.md` and `rust/PARITY.md`. This means the `auto_compact_triggered` harness scenario is a first step, but full behavioral parity for compaction — covering post-compaction health probing, repeated compaction merge quality, boundary preservation under various transcript shapes, and token budget fidelity — is not yet closed.

---

## Parity Documentation

### Top-level PARITY.md

`references/claw-code/PARITY.md` contains:

```
## Runtime Behavioral Gaps
- [x] Permission enforcement across all tools (read-only, workspace-write, danger-full-access)
- [ ] Output truncation (large stdout/file content)
- [ ] Session compaction behavior matching   ← open
- [ ] Token counting / cost tracking accuracy
- [x] Streaming response support validated by the mock parity harness
```

The mock parity harness note for streaming is directly analogous to what compaction needs: the harness provides deterministic evidence that a behavior works. For compaction, the `auto_compact_triggered` scenario provides that evidence for the trigger and format, but the gap remains open because full compaction parity (boundary integrity, summary quality, health probing) is not yet validated by the harness.

### rust/PARITY.md

`references/claw-code/rust/PARITY.md` mirrors the same open items:

```
## Still open
- [ ] Session compaction behavior matching
- [ ] Token counting / cost tracking accuracy
```

The rust workspace parity doc defers to the top-level doc for the authoritative list.

---

## Harness Evidence Chain

The compaction parity evidence forms a coherent chain:

```
ROADMAP.md (intent + dogfooding)
    ↓
PARITY.md (open gap status)
    ↓
mock_parity_scenarios.json (named scenario + parity_refs)
    ↓
mock_parity_harness.rs (assert_auto_compact_triggered + fixture)
    ↓
compact.rs (unit tests for boundary and merge behavior)
```

This is the same pattern used for other parity areas:
- **Permission enforcement** — `bash_permission_prompt_denied`, `bash_permission_prompt_approved`, `write_file_denied` scenarios → `permissions.rs` unit tests
- **Plugin tools** — `plugin_tool_roundtrip` scenario → `mock_parity_harness.rs` assert
- **Streaming** — validated end-to-end by the mock harness (now checked)

For compaction, the chain is partially built: the scenario exists and validates format + trigger, but the behavioral gap remains open because the full compaction contract (boundary integrity, health probe, repeated merge quality) is not yet covered by the harness.

---

## Connection to Other Compaction Documents

The compaction documents in `analysis/` are organized by mechanism layer:

| Document | Covers |
|---|---|
| `compaction-trigger-tail.md` | Token-budget trigger, `should_compact()`, `maybe_auto_compact()`, preserved tail strategy, tool-pair boundary preservation |
| `compaction-resume-packet.md` | Synthetic continuation message anatomy, `resume directly` instruction, summary shape, repeated merge behavior |
| `compaction-parity-evidence.md` | **This document** — roadmap intent, parity gap status, harness evidence chain, what `auto_compact_triggered` proves and what it does not |

The parity document does not re-explain the mechanism — it traces how the mechanism is evidenced and validated. The other two documents explain the mechanism itself.

---

## Builder Lessons

1. **Harness scenarios and parity refs must stay synchronized.** The `auto_compact_triggered` scenario's `parity_refs` array maps to specific PARITY.md gap names. If a gap is renamed in PARITY.md without updating the scenario manifest, the diff checker (`run_mock_parity_diff.py`) will catch the drift — but only if both are kept in sync.

2. **A named scenario is evidence, not proof of completeness.** `auto_compact_triggered` validates the trigger fires and the output shape is correct, but the open-gap marker in PARITY.md correctly signals that full compaction parity (boundary integrity, health probe, summary quality under repeated compaction) is not yet validated by the harness. Claiming full parity based on one passing scenario would be inaccurate.

3. **Dogfooding findings become roadmap items become code changes become test coverage.** The Jobdori session failure (ROADMAP #38) is the clearest example: real usage revealed a failure mode, it was documented as a roadmap item with a specific root cause, the fix was implemented with regression coverage, and the harness scenario validates the trigger — but the health-probe path added in that fix is not yet covered by the harness.

4. **Format parity and behavioral parity are distinct.** The harness validates that `auto_compaction` key appears in JSON output and that the response contains the expected message. It does not validate that the summary quality is good, that tool pairs are not split, or that repeated compaction merges correctly. Those require unit tests and potentially additional scenarios.

5. **The evidence chain must be traceable across all three layers.** When a document claims compaction is validated, it should be able to cite: (a) the roadmap item that motivated it, (b) the PARITY.md gap status, and (c) the specific harness scenario or test that exercises it. Missing any layer means the claim is not fully repo-grounded.
