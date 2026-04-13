# Harness-as-Builder-Pattern: Mock Parity Harness Design Lessons

## What This Document Is

This document explains the mock parity harness as a reusable builder pattern — not just describing what the harness does, but extracting the specific design decisions a builder can transplant into their own test infrastructure. The harness is located at `references/claw-code/rust/crates/rusty-claude-cli/tests/mock_parity_harness.rs` and its supporting mock service at `references/claw-code/rust/crates/mock-anthropic-service/src/lib.rs`.

This document complements `mock-harness-evidence-chain.md` (which traces the four-artifact evidence chain) and `clawable-harness-principles.md` (which covers the seven clawability properties). Here the focus is strictly on what a builder would copy.

---

## The Core Pattern: Scenario Identity in the Request

The most reusable insight from the mock parity harness is **encoding scenario identity into the API request itself** rather than managing per-scenario server configurations.

In `mock-anthropic-service/src/lib.rs`, the mock service detects scenarios by extracting the `model` field from incoming requests:

```rust
pub const SCENARIO_PREFIX: &str = "PARITY_SCENARIO:";

fn detect_scenario(request: &MessageRequest) -> Option<Scenario> {
    request.messages.iter().rev().find_map(|message| {
        message.content.iter().rev().find_map(|block| match block {
            InputContentBlock::Text { text } => text
                .split_whitespace()
                .find_map(|token| token.strip_prefix(SCENARIO_PREFIX))
                .and_then(Scenario::parse),
            _ => None,
        })
    })
}
```

A single Tokio TCP server handles all 12 scenarios. The test passes `PARITY_SCENARIO:streaming_text` as the model name, and the service routes to the `StreamingText` variant without any restart or reconfig. This means:

- **One server process, many scenarios** — no port allocation per scenario, no config file per scenario
- **Scenario identity is a first-class field** — not buried in a side-channel
- **Composable with real traffic** — a real API key and URL can replace the mock by changing two environment variables

**Builder lesson:** When designing a mock service, resist the urge to start a new server per scenario. Instead, encode scenario routing in the request payload. This pattern works for any request field that can carry a string — model name, a custom header, the first message content, or an explicit `scenario` key in the body.

---

## Manifest-Driven Test Case Definition

The harness separates **what scenarios exist** (a JSON manifest) from **how to run them** (Rust test code). This is the second reusable pattern.

The manifest at `references/claw-code/rust/mock_parity_scenarios.json` defines each scenario with name, category, description, and `parity_refs` that link to `PARITY.md`:

```json
{
  "name": "write_file_denied",
  "category": "permissions",
  "description": "Confirms read-only mode blocks write_file with an error result.",
  "parity_refs": [
    "Mock parity harness — milestone 1",
    "Permission enforcement across tool paths"
  ]
}
```

The harness test loads this manifest and cross-checks it against the hardcoded test cases:

```rust
let case_names = cases.iter().map(|case| case.name).collect::<Vec<_>>();
let manifest_names = manifest_entries
    .iter()
    .map(|entry| entry.name.as_str())
    .collect::<Vec<_>>();
assert_eq!(
    case_names, manifest_names,
    "manifest and harness cases must stay aligned"
);
```

This assertion is itself a test — if a developer adds a JSON entry but forgets the Rust case, or vice versa, the test fails at build time. The manifest becomes the **single source of truth for scenario enumeration**.

**Builder lesson:** Keep scenario metadata in a data file, not in test code. A JSON or YAML manifest is auditable without reading Rust, diffable in version control, and processable by external tooling (the Python diff runner). The manifest-to-test alignment assertion catches drift automatically.

---

## Structured JSON Reports for Machine-Readable Assertions

The harness produces structured JSON reports instead of relying on pass/fail exit codes or stdout scraping. Each scenario report includes:

```rust
struct ScenarioReport {
    name: String,
    category: String,
    description: String,
    parity_refs: Vec<String>,
    iterations: u64,
    request_count: usize,
    tool_uses: Vec<String>,
    tool_error_count: usize,
    final_message: String,
}
```

The test writes this to a path specified by `MOCK_PARITY_REPORT_PATH`, and the Python diff runner consumes it:

```rust
fn maybe_write_report(reports: &[ScenarioReport]) {
    let Some(path) = std::env::var_os("MOCK_PARITY_REPORT_PATH") else {
        return;
    };
    // ...
    fs::write(path, serde_json::to_vec_pretty(&payload).expect("report json should serialize"))
        .expect("report should write");
}
```

The diff runner then prints a checklist with pass/mapped/missing status per scenario and a coverage map showing which scenarios exercise which `parity_ref` entries.

**Builder lesson:** Design test outputs as structured data from the start. A JSON report with named fields (`iterations`, `request_count`, `tool_uses`) is far easier to aggregate, diff, and query than parsing prose from stdout. The environment-variable-driven output path (`MOCK_PARITY_REPORT_PATH`) means the same test can write to a file, stdout, or be ignored depending on how it is invoked.

---

## Reference Coverage Checking Prevents Doc Drift

The Python diff runner (`references/claw-code/rust/scripts/run_mock_parity_diff.py`) performs a pre-flight check before running the harness:

```python
def ensure_refs_exist(manifest: list[dict], parity_text: str) -> list[tuple[str, str]]:
    missing: list[tuple[str, str]] = []
    for entry in manifest:
        for ref in entry.get("parity_refs", []):
            if ref not in parity_text:
                missing.append((entry["name"], ref))
    return missing
```

If any `parity_ref` in the manifest does not appear in `PARITY.md`, the script exits with code 1 before running the harness. This closes the **doc-drift gap**: documentation and test coverage cannot silently diverge.

**Builder lesson:** Add a pre-flight check to your CI pipeline that verifies documentation references are still valid. The pattern of `scenario → documentation anchor` mapping is broadly applicable — any time you claim "feature X is covered by test Y," you should have an automated check that the claim is still true when the documentation or tests change.

---

## Captured Requests Prove What the CLI Sent

The mock service captures every inbound request into a `CapturedRequest` struct:

```rust
pub struct CapturedRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub scenario: String,
    pub stream: bool,
    pub raw_body: String,
}
```

The harness asserts on these captured requests to verify the CLI produced the correct API call shape:

```rust
let messages_only: Vec<_> = captured
    .iter()
    .filter(|r| r.path == "/v1/messages")
    .collect();
assert_eq!(messages_only.len(), 21, "twelve scenarios should produce twenty-one /v1/messages requests");
```

The 21 requests across 12 scenarios (some scenarios produce 2 requests because they involve tool use) are verified not just by the CLI's final output, but by the actual HTTP traffic the CLI generated.

**Builder lesson:** Separate **output verification** (did the CLI produce the right JSON?) from **request verification** (did the CLI send the right API call?). Captured request logs let you assert on the intermediate HTTP layer, which catches bugs in request construction that would be invisible if you only checked the final output.

---

## Per-Scenario Prepare/Assert Functions Enable Isolation

Each scenario in the harness has its own `prepare` and `assert` functions:

```rust
ScenarioCase {
    name: "write_file_allowed",
    permission_mode: "workspace-write",
    allowed_tools: Some("write_file"),
    stdin: None,
    prepare: prepare_noop,
    assert: assert_write_file_allowed,
    extra_env: None,
    resume_session: None,
},
```

The `prepare` function sets up the workspace (e.g., creating fixture files), and the `assert` function validates the run:

```rust
fn assert_write_file_allowed(workspace: &HarnessWorkspace, run: &ScenarioRun) {
    assert_eq!(run.response["iterations"], Value::from(2));
    assert_eq!(
        run.response["tool_uses"][0]["name"],
        Value::String("write_file".to_string())
    );
    let generated = workspace.root.join("generated").join("output.txt");
    let contents = fs::read_to_string(&generated).expect("generated file should exist");
    assert_eq!(contents, "generated/output.txt\n");
}
```

**Builder lesson:** Isolate scenario setup and assertions. A `prepare: fn(&HarnessWorkspace)` and `assert: fn(&HarnessWorkspace, &ScenarioRun)` signature lets each scenario bring its own fixtures and validation logic without polluting a shared test function. This keeps the harness extensible — adding a new scenario requires adding an entry to the case array and implementing two functions, not rewriting the test runner.

---

## Ephemeral Workspace Isolation Prevents State Bleed

The harness creates a fresh temporary directory per scenario:

```rust
fn unique_temp_dir(label: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_millis();
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "claw-mock-parity-{label}-{}-{millis}-{counter}",
        std::process::id()
    ))
}
```

Scenarios run sequentially with cleanup between them:

```rust
fs::remove_dir_all(&workspace.root).expect("workspace cleanup should succeed");
```

**Builder lesson:** Use per-scenario ephemeral workspaces. A temp directory per scenario prevents state from leaking between test cases and makes it possible to inspect a failed scenario's workspace after the fact. The cleanup-at-end pattern ensures the harness does not accumulate temp directories on repeated runs.

---

## Deterministic Service Spawning via Ephemeral Port

The mock service binds to an ephemeral port (`127.0.0.1:0`) and reports its actual address:

```rust
pub async fn spawn() -> io::Result<Self> {
    Self::spawn_on("127.0.0.1:0").await
}

pub async fn spawn_on(bind_addr: &str) -> io::Result<Self> {
    let listener = TcpListener::bind(bind_addr).await?;
    let address = listener.local_addr()?;
    // ...
    Ok(Self {
        base_url: format!("http://{address}"),
        // ...
    })
}
```

The test reads `base_url()` and passes it as `ANTHROPIC_BASE_URL`, so no port coordination between processes is needed.

**Builder lesson:** Let the OS allocate the port. Binding to `:0` and reading back `local_addr()` is more robust than hardcoding a port or using a port-file convention. This avoids both port conflicts and the need for a coordination step before starting the test.

---

## Summary of Builder Lessons

| Pattern | Location in Code | What It Prevents |
|---|---|---|
| Scenario prefix in request | `mock-anthropic-service/src/lib.rs` — `detect_scenario` | Per-scenario server processes |
| Manifest-driven test cases | `mock_parity_harness.rs` — `ScenarioCase` array + manifest assertion | Scenario definitions scattered in test code |
| Structured JSON reports | `mock_parity_harness.rs` — `ScenarioReport` + `MOCK_PARITY_REPORT_PATH` | Unparseable stdout-only outputs |
| Reference coverage check | `run_mock_parity_diff.py` — `ensure_refs_exist` | Doc-test drift |
| Captured request logging | `mock-anthropic-service/src/lib.rs` — `CapturedRequest` | Bugs invisible to output-only assertions |
| Per-scenario prepare/assert | `mock_parity_harness.rs` — `ScenarioCase::{prepare, assert}` | Shared state between scenarios |
| Ephemeral workspace | `mock_parity_harness.rs` — `unique_temp_dir` | State bleed across scenarios |
| Ephemeral port binding | `mock-anthropic-service/src/lib.rs` — `spawn_on("127.0.0.1:0")` | Port coordination overhead |

---

## Evidence Files

| Artifact | Path |
|---|---|
| Mock service implementation | `references/claw-code/rust/crates/mock-anthropic-service/src/lib.rs` |
| Harness test | `references/claw-code/rust/crates/rusty-claude-cli/tests/mock_parity_harness.rs` |
| Scenario manifest | `references/claw-code/rust/mock_parity_scenarios.json` |
| Diff/checklist runner | `references/claw-code/rust/scripts/run_mock_parity_diff.py` |
| Harness overview | `references/claw-code/rust/MOCK_PARITY_HARNESS.md` |
