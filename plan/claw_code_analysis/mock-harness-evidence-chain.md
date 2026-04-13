# Mock Harness Evidence Chain

## What This Is

The mock parity harness is a four-artifact validation system that connects a deterministic mock LLM service, a Rust-based end-to-end test, a scenario manifest, and a Python diff/checklist runner into one coherent evidence chain for parity verification. The chain proves behavioral coverage without requiring real API credentials or live upstream services.

## The Four Artifacts

### 1. Deterministic Mock Service — `mock-anthropic-service`

**Location:** `references/claw-code/rust/crates/mock-anthropic-service/src/lib.rs`

The mock service is a Tokio-based TCP server that implements the Anthropic `/v1/messages` interface. It binds to an ephemeral port, accepts connections, and responds with pre-programmed scenario behavior keyed by a `PARITY_SCENARIO:` prefix in the model field:

```rust
pub const SCENARIO_PREFIX: &str = "PARITY_SCENARIO:";
```

When a request arrives, the service extracts the model string and looks for a `PARITY_SCENARIO:` prefix. If present, it switches to the matching scenario behavior — returning specific token streams, tool definitions, and tool-result payloads as scripted for that scenario. If no prefix is found, it falls back to a default streaming response.

The service captures every inbound request into a `CapturedRequest` struct (method, path, headers, scenario name, stream flag, raw body) so the harness can assert on what the CLI actually sent. This captured request log is the primary evidence that the Rust CLI produced the expected API call shape.

Key design lesson: the scenario prefix approach means a single mock server instance can serve many scenarios without restarting or reconfiguring — the scenario identity travels in the request itself.

### 2. End-to-End Harness Test — `mock_parity_harness.rs`

**Location:** `references/claw-code/rust/crates/rusty-claude-cli/tests/mock_parity_harness.rs`

The harness test is a single Rust test function: `clean_env_cli_reaches_mock_anthropic_service_across_scripted_parity_scenarios`. It:

1. Loads the scenario manifest (`mock_parity_scenarios.json`) to get the canonical list of scenario names
2. Spawns `MockAnthropicService::spawn()` on an ephemeral port
3. Sets `ANTHROPIC_BASE_URL` and a dummy `ANTHROPIC_API_KEY` to route traffic to the mock
4. Runs `claw` as a subprocess with controlled env vars and permission mode
5. Captures stdout/stderr and exit code for each scenario
6. Reports structured JSON to a path specified by `MOCK_PARITY_REPORT_PATH`

The test uses a `ScenarioCase` struct with name, permission mode, allowed tools, per-scenario prepare/assert functions, and stdin/extra_env fields. Each scenario has its own assertion logic (e.g., `assert_write_file_denied` checks that a permission-denied tool result appears in the final transcript). The manifest drives scenario selection, not hardcoded test logic — adding a new scenario means adding an entry to the JSON and a matching assert function.

The test writes a JSON report with per-scenario fields: `iterations`, `request_count`, `tool_uses`, `tool_error_count`, and `final_message`. This structured output feeds directly into the diff runner.

### 3. Scenario Manifest — `mock_parity_scenarios.json`

**Location:** `references/claw-code/rust/mock_parity_scenarios.json`

The manifest is the single source of truth for which scenarios exist and what behavioral ground they cover. Each entry has:

```json
{
  "name": "streaming_text",
  "category": "baseline",
  "description": "Validates streamed assistant text with no tool calls.",
  "parity_refs": [
    "Mock parity harness — milestone 1",
    "Streaming response support validated by the mock parity harness"
  ]
}
```

The `parity_refs` field is critical: it links each scenario to specific claims in `PARITY.md`. This is what enables the reference-coverage check in the diff runner. The manifest currently defines 12 scenarios across categories: `baseline`, `file-tools`, `permissions`, `multi-tool-turns`, `bash`, `plugin-paths`, `session-compaction`, and `token-usage`.

The full current manifest includes these scenarios:

| Scenario | Category | Parity Refs Coverage |
|---|---|---|
| `streaming_text` | baseline | Streaming response support validated by the mock parity harness |
| `read_file_roundtrip` | file-tools | File tools — harness-validated flows |
| `grep_chunk_assembly` | file-tools | File tools — harness-validated flows |
| `write_file_allowed` | file-tools | File tools — harness-validated flows |
| `write_file_denied` | permissions | Permission enforcement across tool paths |
| `multi_tool_turn_roundtrip` | multi-tool-turns | Multi-tool assistant turns |
| `bash_stdout_roundtrip` | bash | Bash flow roundtrips |
| `bash_permission_prompt_approved` | permissions | Permission enforcement across tool paths |
| `bash_permission_prompt_denied` | permissions | Permission enforcement across tool paths |
| `plugin_tool_roundtrip` | plugin-paths | Plugin tool execution path |
| `auto_compact_triggered` | session-compaction | Session compaction behavior matching, auto_compaction threshold from env |
| `token_cost_reporting` | token-usage | Token counting / cost tracking accuracy |

### 4. Diff/Checklist Runner — `run_mock_parity_diff.py`

**Location:** `references/claw-code/rust/scripts/run_mock_parity_diff.py`

The Python script orchestrates the full validation loop. Its two primary roles:

**Reference coverage check (pre-flight):** Before running the harness, the script loads the manifest and the full text of `PARITY.md`. It verifies that every `parity_ref` in the manifest actually appears somewhere in `PARITY.md`. If any reference is missing, the script prints a diagnostic and exits with code 1 — this prevents scenario coverage from drifting away from the documented parity claims. This is the reference-coverage checking step required by `VAL-PARITY-004`.

**Harness execution and reporting:** With `--no-run` absent, the script invokes `cargo test -p rusty-claude-cli --test mock_parity_harness -- --nocapture`, passing `MOCK_PARITY_REPORT_PATH` to write the JSON report. It then parses the report and prints a checklist grouped by scenario, showing pass/mapped/missing status, description, parity refs, and for passed scenarios: iterations, request count, tool uses, tool errors, and final message.

The script also prints a PARITY coverage map: which scenarios exercise which `parity_ref` entries. This gives a cross-sectional view of coverage aligned with the parity document structure.

## How the Chain Connects

```
scenario manifest (JSON)          PARITY.md (text)
       |                                |
       |  parity_refs check             |
       v                                |
run_mock_parity_diff.py  ----->  reference validation (exit 1 if missing)
       |
       |  MOCK_PARITY_REPORT_PATH
       v
cargo test mock_parity_harness  -->  spawns mock-anthropic-service
       |                                |
       |  claw CLI under test           |  scenario routing by
       +------------------------------>|  PARITY_SCENARIO: prefix
                                         |
                                         v
                               deterministic scenario responses
                               + captured requests
                                         |
                                         v
                               JSON report written to temp dir
```

**Val-PARITY-002** (connect harness artifacts into one evidence chain): The document above traces how the mock service, harness test, scenario manifest, and diff runner form a single validation system. The mock service provides deterministic responses and request capture; the harness test provides execution infrastructure and structured reporting; the scenario manifest provides named scenario definitions and parity reference anchors; the diff runner provides reference-coverage checking and human-readable checklist output.

**Val-PARITY-003** (use manifest-grounded scenario names): All scenario names cited in this document come directly from `mock_parity_scenarios.json` — `streaming_text`, `read_file_roundtrip`, `grep_chunk_assembly`, `write_file_allowed`, `write_file_denied`, `multi_tool_turn_roundtrip`, `bash_stdout_roundtrip`, `bash_permission_prompt_approved`, `bash_permission_prompt_denied`, `plugin_tool_roundtrip`, `auto_compact_triggered`, `token_cost_reporting`. No paraphrased scenario names appear.

## Builder Lessons

1. **Scenario identity in the request is cleaner than configuration files.** By encoding the scenario name in the model field (`PARITY_SCENARIO:streaming_text`), the mock server avoids a separate config or startup flag for each scenario. The test just passes the right model string per scenario.

2. **A manifest separates scenario definition from test logic.** Adding a new scenario to the harness means adding one JSON entry and one `assert_*` function — not modifying the test runner's core logic. This makes coverage audits a file-read operation rather than a code search.

3. **Reference coverage checking closes the doc-drift gap.** The diff runner's pre-flight check that every `parity_ref` appears in `PARITY.md` ensures that when parity documentation and scenario coverage diverge, the build fails rather than silently drifting.

4. **Structured JSON reports enable machine-readable downstream processing.** By writing `iterations`, `request_count`, `tool_uses`, `tool_error_count`, and `final_message` per scenario, the harness output can feed dashboards, regression tracking, or additional validation layers without parsing freeform text.

## Evidence Files

| Artifact | Path |
|---|---|
| Mock service | `references/claw-code/rust/crates/mock-anthropic-service/src/lib.rs` |
| Harness test | `references/claw-code/rust/crates/rusty-claude-cli/tests/mock_parity_harness.rs` |
| Scenario manifest | `references/claw-code/rust/mock_parity_scenarios.json` |
| Diff runner | `references/claw-code/rust/scripts/run_mock_parity_diff.py` |
| Parity status doc | `references/claw-code/PARITY.md` |
| Harness overview | `references/claw-code/rust/MOCK_PARITY_HARNESS.md` |
